use std::path::Path;
use ts_rs::TS;

fn export_all_protocol_types(out_dir: &Path) -> Result<(), ts_rs::ExportError> {
    use nova_protocol::{agent, errors, llm, misc, rpc, session, settings};

    let cfg = ts_rs::Config::default().with_out_dir(out_dir);

    macro_rules! export_types {
        ($($ty:ty),* $(,)?) => {
            $(<$ty as TS>::export_all(&cfg)?;)*
        };
    }

    export_types!(
        misc::AgentBundle,
        agent::AgentDomainEvent,
        misc::AgentFileEntry,
        llm::AgentMode,
        misc::AgentSource,
        misc::AgentSourceKind,
        agent::AgentTurnSubmitOutcome,
        settings::AppSettings,
        agent::ApprovalPolicy,
        misc::BackgroundOutput,
        llm::ChatMessageEvent,
        rpc::ClientCapabilities,
        rpc::ClientInfo,
        agent::CognitiveState,
        llm::Content,
        llm::ContentBlock,
        session::ConversationMeta,
        misc::ConversationPlan,
        misc::ConversationUsageSummary,
        misc::CronJob,
        misc::FetchedModel,
        misc::FileDiffLine,
        misc::GitRepoStatus,
        misc::HeatmapPoint,
        session::HistoryAttachment,
        session::HistoryMessage,
        session::HistoryToolExecution,
        llm::ImageSource,
        misc::InitGitRepoResult,
        rpc::InitializeParams,
        rpc::InitializeResult,
        misc::ManualCompactOutcome,
        llm::Message,
        misc::ModelBreakdown,
        misc::ModelCatalogStats,
        misc::ModelCatalogSummary,
        errors::NovaErrorInfo,
        misc::PermissionRule,
        settings::ProviderProfile,
        llm::Role,
        misc::RuleKind,
        rpc::ServerCapabilities,
        misc::SandboxStatus,
        misc::SessionFileMeta,
        misc::SkillSummary,
        misc::TerminalInfo,
        misc::TodoEntry,
        misc::TokenUsageRecord,
        llm::Tool,
        misc::TurnTrace,
        agent::TurnSubmitMode,
        agent::TurnSubmitOutcomeKind,
        misc::UsageStats,
        misc::WireCallTrace,
        misc::WorkspaceDiff,
        misc::WorkspaceDirectoryListing,
        misc::WorkspaceEntry,
        misc::WorkspaceFileChange,
        misc::WorkspaceFileContent,
    );

    Ok(())
}

#[test]
fn committed_protocol_ts_matches_live_ts_rs_export() {
    let dir = std::env::temp_dir().join(format!(
        "nova-protocol-sync-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("create temp export dir");

    export_all_protocol_types(&dir).expect("export protocol types");

    let live_path = dir.join("protocol.ts");
    let live = std::fs::read_to_string(&live_path)
        .expect("read live exported protocol.ts")
        .replace("\r\n", "\n");
    let committed = include_str!("../../../src/lib/protocol.ts").replace("\r\n", "\n");

    let _ = std::fs::remove_dir_all(&dir);

    assert_eq!(
        live, committed,
        "src/lib/protocol.ts drifted from nova-protocol; run `npm run gen:bindings`"
    );
}
