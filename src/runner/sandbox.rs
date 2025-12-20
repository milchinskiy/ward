#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SandboxPolicy {
    pub memory_limit_bytes: usize,
    pub instruction_limit: u64,
    pub thread_pool_size: usize,
    #[serde(default, with = "serde_ext_duration::opt")]
    pub timeout: Option<std::time::Duration>,
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        Self {
            memory_limit_bytes: usize::MAX,
            instruction_limit: u64::MAX,
            thread_pool_size: 2,
            timeout: None,
        }
    }
}
