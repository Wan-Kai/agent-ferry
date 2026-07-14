#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RequestKind {
    Ping,
    DetectAgents,
    CreateHandoff,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HostRequest {
    pub request_id: String,
    pub kind: RequestKind,
}
