use rmcp::{
    ServerHandler, ServiceExt,
    model::{Implementation, ServerCapabilities, ServerInfo},
    transport,
};

use crate::error::AdapterResult;

const SERVER_INSTRUCTIONS: &str = "\
引导用户补全日常计划，展示完整中文摘要并取得确认后，\
才能调用执行工具。不得尝试绕过服务器端安全策略。";

#[derive(Debug, Clone, Copy, Default)]
struct AdapterServer;

impl ServerHandler for AdapterServer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo::new(ServerCapabilities::builder().enable_tools().build())
            .with_instructions(SERVER_INSTRUCTIONS)
            .with_server_info(Implementation::new(
                env!("CARGO_PKG_NAME"),
                env!("CARGO_PKG_VERSION"),
            ))
    }
}

pub async fn serve_stdio() -> AdapterResult<()> {
    let service = AdapterServer.serve(transport::stdio()).await?;
    service.waiting().await?;
    Ok(())
}
