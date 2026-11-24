//! Instance identity for clients that talk to several Inkwell servers.

use super::*;

pub(super) async fn instance_info(
    state: &AppState,
    principal: &McpPrincipal,
    args: &Value,
) -> AppResult<Value> {
    require(principal, "write")?;
    let values: Vec<(String, String)> = sqlx::query_as(
        "SELECT name, value FROM settings WHERE name IN ('installation_id', 'runtime_id')",
    )
    .fetch_all(&state.db)
    .await?;
    let values: std::collections::HashMap<String, String> = values.into_iter().collect();
    let installation_id = values
        .get("installation_id")
        .ok_or_else(|| AppError::internal("installation ID is missing"))?;
    let runtime_id = values
        .get("runtime_id")
        .ok_or_else(|| AppError::internal("runtime ID is missing"))?;
    let probe_matches = if let Some(probe) = arg_str(args, "probe") {
        sqlx::query_scalar::<_, i64>(
            "SELECT EXISTS(SELECT 1 FROM settings \
             WHERE name = '__inkwell_import_target_probe__' AND value = ?)",
        )
        .bind(probe)
        .fetch_one(&state.db)
        .await?
            != 0
    } else {
        false
    };
    Ok(json!({
        "installation_id": installation_id,
        "runtime_id": runtime_id,
        "probe_matches": probe_matches
    }))
}
