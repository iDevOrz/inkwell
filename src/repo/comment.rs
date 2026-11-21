use std::collections::HashMap;

use sqlx::{QueryBuilder, Sqlite, SqlitePool};

use crate::error::{AppError, AppResult};
use crate::models::{COMMENT_STATUSES, Comment, CommentNode, CommentWithContent};
use crate::util::now_ts;

#[derive(Debug, Clone)]
pub struct NewComment {
    pub content_id: i64,
    pub parent_id: i64,
    pub author: String,
    pub email: Option<String>,
    pub url: Option<String>,
    pub body: String,
    pub status: String,
    pub ip: Option<String>,
    pub user_agent: Option<String>,
}

pub async fn create(pool: &SqlitePool, input: NewComment) -> AppResult<i64> {
    let author = input.author.trim();
    let body = input.body.trim();
    if author.is_empty() {
        return Err(AppError::bad_request("请填写昵称"));
    }
    if author.chars().count() > 80 {
        return Err(AppError::bad_request("昵称过长"));
    }
    if body.is_empty() {
        return Err(AppError::bad_request("评论内容不能为空"));
    }
    if body.chars().count() > 10_000 {
        return Err(AppError::bad_request("评论内容过长"));
    }
    if !COMMENT_STATUSES.contains(&input.status.as_str()) {
        return Err(AppError::bad_request("非法的评论状态"));
    }

    let email = input
        .email
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty());
    if let Some(email) = email
        && (email.chars().count() > 150 || !email.contains('@'))
    {
        return Err(AppError::bad_request("邮箱格式不正确"));
    }

    let url = input.url.as_deref().map(str::trim).filter(|value| {
        !value.is_empty()
            && value.chars().count() <= 255
            && (value.starts_with("http://") || value.starts_with("https://"))
    });

    let result = sqlx::query(
        "INSERT INTO comments(content_id, parent_id, author, email, url, body, status, ip, user_agent, created_at) \
         VALUES(?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(input.content_id)
    .bind(input.parent_id.max(0))
    .bind(author)
    .bind(email)
    .bind(url)
    .bind(body)
    .bind(&input.status)
    .bind(input.ip.as_deref())
    .bind(input.user_agent.as_deref())
    .bind(now_ts())
    .execute(pool)
    .await?;

    Ok(result.last_insert_rowid())
}

pub async fn get(pool: &SqlitePool, id: i64) -> AppResult<Option<Comment>> {
    Ok(sqlx::query_as::<_, Comment>(
        "SELECT id, content_id, parent_id, author, email, url, body, status, ip, user_agent, created_at \
         FROM comments WHERE id = ?",
    )
    .bind(id)
    .fetch_optional(pool)
    .await?)
}

pub async fn reply_depth(pool: &SqlitePool, comment_id: i64) -> AppResult<i64> {
    Ok(sqlx::query_scalar(
        "WITH RECURSIVE ancestry(id, parent_id, depth) AS ( \
             SELECT id, parent_id, 0 FROM comments WHERE id = ? \
             UNION ALL \
             SELECT parent.id, parent.parent_id, ancestry.depth + 1 \
             FROM comments parent JOIN ancestry ON parent.id = ancestry.parent_id \
             WHERE ancestry.parent_id > 0 AND ancestry.depth < 64 \
         ) \
         SELECT COALESCE(MAX(depth), 0) FROM ancestry",
    )
    .bind(comment_id)
    .fetch_one(pool)
    .await?)
}

pub async fn list_for_content(
    pool: &SqlitePool,
    content_id: i64,
    status: &str,
) -> AppResult<Vec<Comment>> {
    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT id, content_id, parent_id, author, email, url, body, status, ip, user_agent, created_at \
         FROM comments WHERE content_id = ",
    );
    query.push_bind(content_id);
    if status != "all" {
        query.push(" AND status = ").push_bind(status.to_string());
    }
    query.push(" ORDER BY created_at ASC, id ASC");
    Ok(query.build_query_as::<Comment>().fetch_all(pool).await?)
}

pub fn build_tree(comments: Vec<Comment>) -> Vec<CommentNode> {
    let len = comments.len();
    if len == 0 {
        return Vec::new();
    }

    let indices: HashMap<i64, usize> = comments
        .iter()
        .enumerate()
        .map(|(index, comment)| (comment.id, index))
        .collect();
    let mut parents: Vec<Option<usize>> = comments
        .iter()
        .map(|comment| {
            indices
                .get(&comment.parent_id)
                .copied()
                .filter(|parent| *parent != indices[&comment.id])
        })
        .collect();

    // Break malformed cycles defensively so traversal and drop remain bounded.
    let mut completed = vec![false; len];
    for start in 0..len {
        if completed[start] {
            continue;
        }
        let mut path = Vec::new();
        let mut positions = HashMap::new();
        let mut current = Some(start);
        while let Some(index) = current {
            if completed[index] {
                break;
            }
            if positions.contains_key(&index) {
                parents[index] = None;
                break;
            }
            positions.insert(index, path.len());
            path.push(index);
            current = parents[index];
        }
        for index in path {
            completed[index] = true;
        }
    }

    let mut children = vec![Vec::new(); len];
    let mut roots = Vec::new();
    for (index, parent) in parents.into_iter().enumerate() {
        if let Some(parent) = parent {
            children[parent].push(index);
        } else {
            roots.push(index);
        }
    }

    let mut comments: Vec<Option<Comment>> = comments.into_iter().map(Some).collect();
    let mut built: Vec<Option<CommentNode>> = (0..len).map(|_| None).collect();
    for root in &roots {
        let mut stack = vec![(*root, false)];
        while let Some((index, expanded)) = stack.pop() {
            if built[index].is_some() {
                continue;
            }
            if expanded {
                let node_children = children[index]
                    .iter()
                    .filter_map(|child| built[*child].take())
                    .collect();
                built[index] = Some(CommentNode {
                    comment: comments[index].take().expect("comment node is built once"),
                    children: node_children,
                });
            } else {
                stack.push((index, true));
                for child in children[index].iter().rev() {
                    stack.push((*child, false));
                }
            }
        }
    }

    roots
        .into_iter()
        .filter_map(|index| built[index].take())
        .collect()
}

pub async fn list_admin(
    pool: &SqlitePool,
    status: &str,
    limit: i64,
    offset: i64,
) -> AppResult<(Vec<CommentWithContent>, i64)> {
    let mut count = QueryBuilder::<Sqlite>::new("SELECT COUNT(*) FROM comments cm WHERE 1=1");
    if status != "all" {
        count
            .push(" AND cm.status = ")
            .push_bind(status.to_string());
    }
    let total: i64 = count.build_query_scalar().fetch_one(pool).await?;

    let mut query = QueryBuilder::<Sqlite>::new(
        "SELECT cm.id, cm.content_id, cm.parent_id, cm.author, cm.email, cm.url, cm.body, \
                cm.status, cm.ip, cm.user_agent, cm.created_at, \
                c.title AS content_title, c.slug AS content_slug, c.kind AS content_kind \
         FROM comments cm JOIN contents c ON c.id = cm.content_id WHERE 1=1",
    );
    if status != "all" {
        query
            .push(" AND cm.status = ")
            .push_bind(status.to_string());
    }
    query
        .push(" ORDER BY cm.created_at DESC, cm.id DESC LIMIT ")
        .push_bind(limit.clamp(1, 200))
        .push(" OFFSET ")
        .push_bind(offset.max(0));

    let rows = query
        .build_query_as::<CommentWithContent>()
        .fetch_all(pool)
        .await?;
    Ok((rows, total))
}

pub async fn moderate(pool: &SqlitePool, id: i64, status: &str) -> AppResult<()> {
    if !COMMENT_STATUSES.contains(&status) {
        return Err(AppError::bad_request("非法的评论状态"));
    }
    sqlx::query("UPDATE comments SET status = ? WHERE id = ?")
        .bind(status)
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn delete(pool: &SqlitePool, id: i64) -> AppResult<()> {
    sqlx::query("DELETE FROM comments WHERE id = ?")
        .bind(id)
        .execute(pool)
        .await?;
    Ok(())
}

pub async fn count_by_status(pool: &SqlitePool) -> AppResult<HashMap<String, i64>> {
    let rows: Vec<(String, i64)> =
        sqlx::query_as("SELECT status, COUNT(*) FROM comments GROUP BY status")
            .fetch_all(pool)
            .await?;
    Ok(rows.into_iter().collect())
}

pub async fn count_pending(pool: &SqlitePool) -> AppResult<i64> {
    Ok(
        sqlx::query_scalar("SELECT COUNT(*) FROM comments WHERE status = 'pending'")
            .fetch_one(pool)
            .await?,
    )
}

pub async fn count_all(pool: &SqlitePool) -> AppResult<i64> {
    Ok(sqlx::query_scalar("SELECT COUNT(*) FROM comments")
        .fetch_one(pool)
        .await?)
}

pub async fn recent_approved(pool: &SqlitePool, limit: i64) -> AppResult<Vec<CommentWithContent>> {
    Ok(sqlx::query_as::<_, CommentWithContent>(
        "SELECT cm.id, cm.content_id, cm.parent_id, cm.author, cm.email, cm.url, cm.body, \
                cm.status, cm.ip, cm.user_agent, cm.created_at, \
                c.title AS content_title, c.slug AS content_slug, c.kind AS content_kind \
         FROM comments cm JOIN contents c ON c.id = cm.content_id \
         WHERE cm.status = 'approved' ORDER BY cm.created_at DESC LIMIT ?",
    )
    .bind(limit.clamp(1, 50))
    .fetch_all(pool)
    .await?)
}
