#[tokio::main]
async fn main() -> maa_mcp_adapter::error::AdapterResult<()> {
    maa_mcp_adapter::server::serve_stdio().await
}
