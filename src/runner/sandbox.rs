#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct SandboxPolicy {
    pub memory_limit_bytes: usize,
    pub instruction_limit: u64,
    pub allow_fs_write: bool,
    pub allow_env_mutation: bool,
    pub allow_require: bool,
    pub thread_pool_size: usize,
    #[serde(default, with = "serde_ext_duration::opt")]
    pub timeout: Option<std::time::Duration>,
    pub ward_modules: std::collections::HashMap<String, bool>,
    pub globals: std::collections::HashMap<String, bool>,
}

impl Default for SandboxPolicy {
    fn default() -> Self {
        Self {
            memory_limit_bytes: 10_000_000,
            instruction_limit: 100_000_000,
            allow_fs_write: true,
            allow_env_mutation: true,
            allow_require: true,
            thread_pool_size: 2,
            timeout: None,
            ward_modules: std::collections::HashMap::from([
                ("convert".to_string(), true),
                ("env".to_string(), true),
                ("fs".to_string(), true),
                ("helpers".to_string(), true),
                ("http".to_string(), true),
                ("io".to_string(), true),
            ]),
            globals: std::collections::HashMap::from([
                // lua standard library
                ("assert".to_string(), true),
                ("error".to_string(), true),
                ("ipairs".to_string(), true),
                ("next".to_string(), true),
                ("pairs".to_string(), true),
                ("pcall".to_string(), true),
                ("select".to_string(), true),
                ("tonumber".to_string(), true),
                ("tostring".to_string(), true),
                ("type".to_string(), true),
                ("xpcall".to_string(), true),
                ("setmetatable".to_string(), true),
                ("getmetatable".to_string(), true),
                ("rawequal".to_string(), true),
                ("rawget".to_string(), true),
                ("rawset".to_string(), true),
                ("print".to_string(), true),
                ("warn".to_string(), true),
                // lua features, enabled by default
                ("coroutine".to_string(), true),
                ("math".to_string(), true),
                ("string".to_string(), true),
                ("table".to_string(), true),
                ("utf8".to_string(), true),
                // known to be unsafe, disabled by default
                ("dofile".to_string(), false),
                ("load".to_string(), false),
                ("loadfile".to_string(), false),
            ]),
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct SandboxPolicyPermissions {
    pub allow_fs_write: bool,
    pub allow_env_mutation: bool,
}

impl From<&SandboxPolicy> for SandboxPolicyPermissions {
    fn from(policy: &SandboxPolicy) -> Self {
        Self {
            allow_fs_write: policy.allow_fs_write,
            allow_env_mutation: policy.allow_env_mutation,
        }
    }
}
