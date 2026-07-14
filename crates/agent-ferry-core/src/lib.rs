#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LaunchMode {
    Terminal,
    Managed,
    Background,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CapturedSource {
    pub title: String,
    pub url: String,
    pub markdown: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Handoff {
    pub source: CapturedSource,
    pub objective: String,
    pub workspace_id: String,
    pub agent_id: String,
    pub launch_mode: LaunchMode,
}
