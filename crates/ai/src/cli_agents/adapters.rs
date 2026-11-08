//! How each agent CLI is driven: its arguments, the flags it must have, how its output stream
//! reads, and anything it needs before a run (a sign-in check). Each is an [`AgentAdapter`]; an
//! [`AgentAdapters`] registry maps agent ids to them, and an agent it doesn't know gets the plain
//! adapter (prompt in, text out). Add an agent by implementing the trait and registering it.

use super::*;

mod aider;
mod antigravity;
mod copilot;
mod dsh;
mod grok;
mod hermes;
mod maki;
mod opencode;
pub use aider::Aider;
pub use antigravity::Antigravity;
pub use copilot::Copilot;
pub use dsh::DeepSeekHarness;
pub use grok::Grok;
pub use hermes::Hermes;
pub use maki::Maki;
pub use opencode::OpenCode;

/// What a run's arguments are built from.
pub struct ArgsCx<'a> {
    pub options: &'a BackendOptions,
    /// The message, already protected from being read as a flag.
    pub prompt: &'a str,
    pub preamble: &'a str,
    pub mcp: &'a McpCommand,
    /// A saved session to continue (only passed to adapters that [`resume`](AgentAdapter::resumes)).
    pub resume: Option<&'a str>,
    /// Build mode: the agent may edit files; approvals and questions come to us.
    pub builder: bool,
    /// Clean room: reads go through our approval tool too, so the original stays out of reach.
    pub reads_checked: bool,
    /// A question about a build: only reading (none in Clean room), no tool server, and a fresh
    /// session or a fork that leaves the builder's session unchanged.
    pub question: bool,
}

/// One way of driving an agent CLI.
pub trait AgentAdapter: Send + Sync {
    /// The agent id this adapter is registered under (`claude-code`), plus any other names.
    fn ids(&self) -> &'static [&'static str];
    /// argv after the program for one message.
    fn args(&self, cx: &ArgsCx) -> Vec<String>;
    /// Whether this agent supplies its own live model and thinking choices.
    fn has_model_catalog(&self) -> bool {
        false
    }

    fn model_catalog_depends_on_selection(&self) -> bool {
        false
    }
    /// Query models visible in this backend's execution context, respecting cancellation.
    /// Agents without catalogue support return an empty list.
    fn model_catalog(
        &self,
        _config: &BackendConfig,
        _options: &BackendOptions,
        _directory: Option<&Path>,
        _stop: &Stop,
    ) -> Result<Vec<models::AgentModel>, ToolError> {
        Ok(Vec::new())
    }
    /// Check a saved choice in the actual run context before invoking the agent.
    fn validate_model(&self, _config: &BackendConfig, _options: &BackendOptions, _directory: Option<&Path>, _stop: &Stop) -> Result<(), ToolError> {
        Ok(())
    }
    /// Refuse options that the native transport cannot support, before preflight or process I/O.
    /// Saving a model choice and discovering its catalogue do not imply native execution support.
    fn check_native_options(&self, _options: &BackendOptions) -> Result<(), ToolError> {
        Ok(())
    }

    /// Refuse saved native tools that this run mode cannot use. The default allows them.
    fn check_saved_tools(&self, _options: &BackendOptions, _allow_helpers: bool) -> Result<(), ToolError> {
        Ok(())
    }

    /// Refuse a custom helper that would act without the approval tool. The default has none.
    /// `project` is the folder the agent runs in; `config` says whether it runs on another computer.
    fn check_subagent_modes(&self, _config: &BackendConfig, _options: &BackendOptions, _project: Option<&Path>) -> Result<(), ToolError> {
        Ok(())
    }

    /// Use ACP by default and refuse the per-message CLI runner. An explicit `acp = false` or
    /// an SSH host must fail rather than opening an interactive agent with no approval channel.
    fn requires_acp(&self) -> bool {
        false
    }
    /// Startup arguments before the ACP flags: models, effort and the tools this access mode
    /// may expose. The default keeps the backend's extra arguments unchanged.
    fn acp_options(&self, options: &BackendOptions, _access: Access) -> Vec<String> {
        options.args.clone()
    }
    /// Session-only MCP startup flags for an agent that needs more than `session/new`'s server
    /// list. Questions never receive these flags, and the person's saved config isn't changed.
    fn acp_mcp_options(&self, _mcp: &McpCommand) -> Vec<String> {
        Vec::new()
    }
    /// Include `"type": "stdio"` on the MCP server in `session/new`. Grok's server enum requires
    /// it. Other agents leave it off, so a session that has not been checked with the field
    /// stays on the payload it already accepts.
    fn acp_mcp_stdio_type(&self) -> bool {
        false
    }
    /// Arguments after the ACP mode words and the model flag, still before a trailing subcommand.
    /// The default adds none, so agents whose flags already sit in [`acp_options`](Self::acp_options)
    /// stay unchanged.
    fn acp_mode_options(&self, _options: &BackendOptions, _access: Access) -> Vec<String> {
        Vec::new()
    }
    /// Words after every other ACP argument. Grok Build needs `stdio` last.
    fn acp_trailing(&self) -> &'static [&'static str] {
        &[]
    }
    /// Session options applied with `session/set_config_option` after the session opens, in order,
    /// before the conversation is ready. Each value is sent as a JSON string. The default sets none.
    fn acp_session_config(&self, _options: &BackendOptions) -> Vec<(String, String)> {
        Vec::new()
    }
    /// Refuse backend options that could bypass this adapter's restrictions, before its ACP
    /// process starts. The refusal is kept on the conversation and reported by `send`.
    fn validate_options(&self, _options: &BackendOptions) -> Result<(), ToolError> {
        Ok(())
    }
    /// Refuse an access mode this agent cannot hold. The default allows every mode. A refusal
    /// is kept beside [`validate_options`](Self::validate_options) and reported by `send`.
    /// Model-list discovery sends no prompt, so it does not apply this check.
    fn validate_access(&self, _access: Access) -> Result<(), ToolError> {
        Ok(())
    }
    /// The model shown in Settings: an explicit choice or the CLI's own default. `None` means
    /// the agent picks its model and doesn't offer a model selector here.
    fn model_in_use(&self, _options: Option<&BackendOptions>) -> Option<String> {
        None
    }
    /// Models the person can choose, from a catalogue or a bounded agent query. May block:
    /// call off the UI thread. Empty means this agent offers no choices.
    fn model_choices(&self, _config: &BackendConfig, _options: &BackendOptions) -> Result<Vec<String>, ToolError> {
        Ok(Vec::new())
    }
    /// Whether this adapter offers context and reasoning controls beyond the model selector.
    fn supports_model_tuning(&self) -> bool {
        false
    }
    /// `(help invocation, substrings it must mention)`, checked once before the first run; when
    /// they're missing the run falls back to the plain adapter. Empty: nothing to check.
    fn required_flags(&self) -> (&'static [&'static str], &'static [&'static str]) {
        (&[], &[])
    }
    /// `program <prompt>` answers and exits, so the plain adapter is a fallback. When it opens an
    /// interactive session instead, a failed flag check keeps this adapter, and the run says what
    /// went wrong (#199).
    fn plain_fallback(&self) -> bool {
        true
    }
    /// The CLI writes one JSON event per line (else its output is the answer text).
    fn streams_json(&self) -> bool {
        true
    }
    /// Events for one parsed JSON line.
    fn parse(&self, _p: &mut StreamParser, _v: &Value) -> Vec<AgentEvent> {
        Vec::new()
    }
    /// It can continue a saved session.
    fn resumes(&self) -> bool {
        false
    }
    /// It honours [`ArgsCx::question`]: it can be held to reading only, using a fresh session
    /// or leaving a resumed session unchanged. Questions go only to adapters that say so.
    fn answers_questions(&self) -> bool {
        false
    }
    /// The message goes in on stdin, not in argv: a long one passes Windows' command-line limit
    /// ("The filename or extension is too long", os error 206, #402).
    fn prompt_on_stdin(&self) -> bool {
        false
    }
    /// What goes in on stdin when [`prompt_on_stdin`](AgentAdapter::prompt_on_stdin) says so:
    /// the message, with the preamble in front for an agent that takes no separate one.
    fn stdin_text(&self, _preamble: &str, prompt: &str) -> String {
        prompt.to_string()
    }
    /// The process for a local run of `program` (an adapter may clean its environment).
    fn process(&self, program: &str) -> Process {
        Process::new(program)
    }
    /// Whether it's signed in, checked before a build starts. `None`: can't tell (the run itself
    /// reports any sign-in problem).
    /// Whether it still names an account it was signed in to, so a sign-in that doesn't work any
    /// more has expired (or was signed out) rather than never happened.
    fn signed_in_before(&self) -> bool {
        false
    }
    fn signed_in(&self, _program: &str) -> Option<bool> {
        None
    }
    /// The plain-words fix when sign-in preflight or ACP startup fails. Login stays in the CLI.
    fn sign_in_fix(&self) -> Option<String> {
        None
    }
    /// Whether what a build needs from this computer is there, for an agent that's handed our tool
    /// server, checked before a build starts. An agent that isn't handed it has nothing to check.
    /// `stop` ends the check when the person stops the build.
    fn check_tool_server(&self, _stop: &CancelToken) -> Result<(), ToolError> {
        Ok(())
    }
    /// An environment variable that hands a local native run our tool server, for an agent that
    /// takes it from its environment rather than its arguments.
    fn mcp_env(&self, _mcp: &McpCommand) -> Result<Option<(&'static str, String)>, ToolError> {
        Ok(None)
    }
    /// Whether what a build needs on a remote SSH host is there, checked before a build starts.
    /// Default: nothing to check on the host. `stop` ends the check when the person stops the build.
    fn check_remote_tool_server(&self, _host: &str, _stop: &CancelToken) -> Result<(), ToolError> {
        Ok(())
    }
    /// Prepare this run's tool server on a remote SSH host (a config file there), returning the
    /// environment variable to set on the remote command. Default: the agent can't carry one.
    fn remote_tool_server(&self, _host: &str, _mcp: &McpCommand) -> Result<Option<(&'static str, String)>, ToolError> {
        Ok(None)
    }
    /// The file in which this computer keeps the agent's saved session `id`, so the project can
    /// keep a copy that travels with it. `None`: it keeps none we can copy, or not here.
    fn session_file(&self, _id: &str) -> Option<std::path::PathBuf> {
        None
    }
    /// What a new session needs to know of an earlier one, read from a kept copy of its
    /// [`session_file`](AgentAdapter::session_file): what it was asked and what it said, the
    /// newest kept, within `max` characters. `None`: nothing to tell.
    fn handoff(&self, _saved: &str, _max: usize) -> Option<String> {
        None
    }
    /// Put a kept copy of session `id` back where the agent looks for it, for a run in `cwd`,
    /// when this computer doesn't have it, so the same session carries on. `Ok(true)`: put
    /// back; `Ok(false)`: nothing to do (it's here already, or this agent keeps none).
    fn restore_session(&self, _id: &str, _saved: &[u8], _cwd: &std::path::Path) -> Result<bool, ToolError> {
        Ok(false)
    }
    /// The environment variable that moves the agent's own folder (its sign-in, settings and
    /// sessions), for [`BackendOptions::config_dir`]. `None`: it has none, and the setting is
    /// ignored.
    fn folder_var(&self) -> Option<&'static str> {
        None
    }
    /// This adapter for the agent kept in `folder` on this computer: it runs there, checks the
    /// sign-in there and keeps its sessions there. `None` when it has no such folder.
    fn in_folder(&self, _folder: std::path::PathBuf) -> Option<Arc<dyn AgentAdapter>> {
        None
    }
}

/// The adapters a run can use.
#[derive(Clone)]
pub struct AgentAdapters {
    adapters: Vec<Arc<dyn AgentAdapter>>,
}

impl std::fmt::Debug for AgentAdapters {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_list().entries(self.adapters.iter().map(|a| a.ids().first().copied().unwrap_or_default())).finish()
    }
}

impl Default for AgentAdapters {
    fn default() -> Self {
        AgentAdapters::built_in()
    }
}

impl AgentAdapters {
    /// The built-in coding agents, including the ACP-only Copilot and Grok Build adapters.
    pub fn built_in() -> AgentAdapters {
        AgentAdapters {
            adapters: vec![
                Arc::new(ClaudeCode::default()),
                Arc::new(Codex),
                Arc::new(GeminiCli),
                Arc::new(OpenCode),
                Arc::new(Antigravity),
                Arc::new(Hermes),
                Arc::new(Maki),
                Arc::new(Aider),
                Arc::new(Copilot),
                Arc::new(Grok),
                Arc::new(DeepSeekHarness),
            ],
        }
    }

    /// Add an adapter; it takes over any ids it shares with one already registered.
    pub fn register(&mut self, adapter: Arc<dyn AgentAdapter>) {
        self.adapters.retain(|a| !a.ids().iter().any(|id| adapter.ids().contains(id)));
        self.adapters.push(adapter);
    }

    /// The adapter for an agent id; one it doesn't know gets the plain adapter.
    pub fn for_agent(&self, agent: &str) -> Arc<dyn AgentAdapter> {
        self.adapters.iter().find(|a| a.ids().contains(&agent)).cloned().unwrap_or_else(|| Arc::new(Plain))
    }

    /// The adapter for an agent with a backend's options: in the folder they choose for it, when
    /// it keeps one ([`AgentAdapter::in_folder`]).
    pub fn for_backend(&self, agent: &str, options: &BackendOptions) -> Arc<dyn AgentAdapter> {
        let adapter = self.for_agent(agent);
        options.config_dir_here().and_then(|dir| adapter.in_folder(dir)).unwrap_or(adapter)
    }
}

/// Any CLI: the prompt as its last argument, its output streamed as text.
pub struct Plain;

impl AgentAdapter for Plain {
    fn ids(&self) -> &'static [&'static str] {
        &[]
    }
    fn args(&self, cx: &ArgsCx) -> Vec<String> {
        let mut a = cx.options.args.clone();
        a.push(format!("{}{}", cx.preamble, cx.prompt));
        a
    }
    fn streams_json(&self) -> bool {
        false
    }
}

/// Claude Code settings with one PostToolUse hook, run after every tool call: our CLI hands over
/// the notes waiting in `dir` (`craft-anything-cli builder-notes <dir>`). Paths use forward
/// slashes and quotes so the hook's shell reads them the same on every platform.
pub fn notes_hook_settings(cli: &str, dir: &std::path::Path) -> String {
    let fwd = |s: &str| s.replace('\\', "/");
    let command = format!("\"{}\" builder-notes \"{}\"", fwd(cli), fwd(&dir.display().to_string()));
    json!({"hooks": {"PostToolUse": [{"matcher": "*", "hooks": [{"type": "command", "command": command, "timeout": 10}]}]}}).to_string()
}

/// Claude Code's tools that change something, reach outside, or start helpers, refused for a
/// question. `Task` is the pre-2.1.63 name of `Agent`; both are kept so older CLIs are held too.
const CHANGING_TOOLS: &[&str] =
    &["Write", "Edit", "MultiEdit", "NotebookEdit", "Bash", "PowerShell", "WebFetch", "WebSearch", "Task", "Agent", "TodoWrite", "AskUserQuestion"];

/// Claude Code's tools that read files, refused outright in Clean room (#564): its defaults let it
/// read in its own folder and refuse elsewhere only until a release or a setting changes that.
const READING_TOOLS: &[&str] = &["Read", "Glob", "Grep", "LS", "NotebookRead"];

/// The built-in tools a builder gets. `Agent` is Claude Code 2.1.63's name for the subagent tool
/// (it was `Task`); a Deep build gets it so the builder can hand work to cheaper helpers. Spawning
/// needs no permission; Claude Code documents that a subagent's own calls are checked against the
/// session's permission rules. Clean room leaves it out for now (#1105).
const BUILDER_TOOLS: &str = "Bash,PowerShell,Read,Write,Edit,MultiEdit,Glob,Grep,TodoWrite,NotebookEdit";
const DEEP_BUILDER_TOOLS: &str = "Agent,Bash,PowerShell,Read,Write,Edit,MultiEdit,Glob,Grep,TodoWrite,NotebookEdit";

/// A remote Deep build's `--tools` list without `Agent`: its custom agents are on the other
/// computer, where the local preflight can't read their permission modes.
pub(crate) fn without_agent_tools(args: &mut [String]) {
    // Every `--tools`, including one a saved backend's extra arguments already carry: a single
    // strip would leave our Agent-bearing list in the remote command.
    for i in 0..args.len() {
        if args[i] == "--tools"
            && let Some(tools) = args.get_mut(i + 1)
        {
            *tools = tools.split(',').filter(|tool| *tool != "Agent" && *tool != "Task").collect::<Vec<_>>().join(",");
        }
    }
}

/// Claude Code answering a question about a build: the builder's session copied
/// (`--fork-session`) and the copy not kept, no tool server, and only Read, Glob and Grep (none in
/// Clean room).
fn question_args(mut a: Vec<String>, cx: &ArgsCx) -> Vec<String> {
    a.extend([
        "-p".into(),
        "--output-format".into(),
        "stream-json".into(),
        "--verbose".into(),
        "--append-system-prompt".into(),
        cx.preamble.to_string(),
        "--model".into(),
        cx.options.cli_model.clone().unwrap_or_else(|| crate::anthropic::DEFAULT_MODEL.into()),
    ]);
    if let Some(r) = cx.resume {
        a.extend(["--resume".into(), r.to_string(), "--fork-session".into()]);
    }
    a.extend(["--no-session-persistence".into(), "--strict-mcp-config".into(), "--disallowedTools".into()]);
    a.extend(CHANGING_TOOLS.iter().map(|t| t.to_string()));
    if cx.reads_checked {
        a.extend(READING_TOOLS.iter().map(|t| t.to_string()));
    } else {
        a.extend(["--allowedTools", "Read", "Glob", "Grep"].map(String::from));
    }
    a
}

/// How Claude Code's result names a `--resume` session it doesn't have (2.1: `errors`, with no
/// turn taken). Its sessions are kept per computer and per folder, so a project copied elsewhere
/// has an id it can't find.
const NO_SUCH_SESSION: &str = "No conversation found with session ID";

/// A failed result that only says the session to resume isn't there.
fn no_such_session(v: &Value) -> bool {
    let named = |e: &Value| e.as_str().is_some_and(|e| e.contains(NO_SUCH_SESSION));
    let turns = v.get("num_turns").and_then(Value::as_u64).unwrap_or(0);
    turns == 0 && (v.get("errors").and_then(Value::as_array).is_some_and(|a| a.iter().any(named)) || v.get("result").is_some_and(named))
}

/// Claude Code in headless mode (flags checked against `claude --help`).
#[derive(Default)]
pub struct ClaudeCode {
    /// The folder chosen for it ([`BackendOptions::config_dir`]); `None`: its usual one.
    folder: Option<std::path::PathBuf>,
}

impl ClaudeCode {
    /// The folder it keeps its sign-in and sessions in: the chosen one, else its usual one.
    fn config_dir(&self) -> Option<std::path::PathBuf> {
        self.folder.clone().or_else(claude_config_dir)
    }
}

/// The most of an agent definition we read while checking: its YAML frontmatter is at the top.
const AGENT_FILE_CAP: u64 = 64 * 1024;
/// Bounds for the nonrecursive event parser: a hostile frontmatter can't recurse or flood it.
const MAX_YAML_DEPTH: usize = 16;
const MAX_YAML_EVENTS: usize = 4096;
/// A custom subagent's permission modes that may apply edits without asking us. A mode this build
/// doesn't know is left to Claude Code's own default.
const PERMISSIVE_MODES: &[&str] = &["acceptEdits", "bypassPermissions", "auto", "plan"];

/// Whether saved `--tools` values name a helper tool. Both `<--tools>` and `<--tools=...>` forms
/// are comma lists.
fn saved_tools_include_agents(args: &[String]) -> bool {
    let mut args = args.iter();
    while let Some(arg) = args.next() {
        let list = if arg == "--tools" { args.next().map(String::as_str) } else { arg.strip_prefix("--tools=") };
        if list.is_some_and(|list| {
            list.split(',').any(|tool| {
                let tool = tool.trim();
                matches!(tool, "Agent" | "Task") || tool.starts_with("Agent(") || tool.starts_with("Task(")
            })
        }) {
            return true;
        }
    }
    false
}

/// A relative config folder means the child's working folder, not ours: Claude Code resolves
/// `CLAUDE_CONFIG_DIR` and a saved relative folder after changing into the project.
fn child_path(path: Option<std::path::PathBuf>, project: Option<&Path>) -> Option<std::path::PathBuf> {
    path.map(|path| if path.is_absolute() { path } else { project.map(|root| root.join(&path)).unwrap_or(path) })
}

/// The first custom subagent whose own permission mode would accept edits without asking the
/// project's approval tool. Claude Code loads agents from `agents/` in its config folder,
/// `.claude/agents/` in the managed settings directory, and `.claude/agents/` from the project up
/// to its repository root, recursively.
fn permissive_subagent(config_dir: Option<&std::path::Path>, project: Option<&std::path::Path>) -> Result<Option<(std::path::PathBuf, String)>, ToolError> {
    let mut visited = std::collections::HashSet::new();
    for dir in agent_dirs(config_dir, project) {
        if let Some(found) = scan_agent_dir(&dir, 0, &mut visited)? {
            return Ok(Some(found));
        }
    }
    Ok(None)
}

/// Claude Code's managed subagents: `.claude/agents/` inside the file-based managed settings
/// directory. Paths per the managed-settings docs: Linux/WSL `/etc/claude-code`, macOS
/// `/Library/Application Support/ClaudeCode`, Windows `C:\\Program Files\\ClaudeCode` (the legacy
/// `C:\\ProgramData\\ClaudeCode` is not read).
fn managed_agents_dir() -> std::path::PathBuf {
    let root = if cfg!(target_os = "macos") {
        std::path::PathBuf::from("/Library/Application Support/ClaudeCode")
    } else if cfg!(target_os = "windows") {
        std::path::PathBuf::from(r"C:\Program Files\ClaudeCode")
    } else {
        std::path::PathBuf::from("/etc/claude-code")
    };
    root.join(".claude").join("agents")
}

/// The user config's `agents/`, the managed settings `.claude/agents/`, the project's
/// `.claude/agents/`, and every `.claude/agents/` between the project and its repository root,
/// as Claude Code discovers subagents.
fn agent_dirs(config_dir: Option<&std::path::Path>, project: Option<&std::path::Path>) -> Vec<std::path::PathBuf> {
    let mut dirs = vec![managed_agents_dir()];
    if let Some(dir) = config_dir {
        dirs.push(dir.join("agents"));
    }
    if let Some(root) = project {
        // Claude Code discovers from its working directory after the OS resolves it; match that
        // rather than a lexical path that may contain `..` or a symlinked component.
        let root = std::fs::canonicalize(root).unwrap_or_else(|_| root.to_path_buf());
        let repo = root.ancestors().find(|dir| dir.join(".git").exists());
        let stop = repo.unwrap_or(root.as_path());
        for dir in root.ancestors() {
            dirs.push(dir.join(".claude").join("agents"));
            if dir == stop {
                break;
            }
        }
    }
    dirs
}

/// One `agents/` directory, its subfolders and their `.md` files. A file or folder that can't be
/// read fails the check rather than hiding a permissive agent; a symlink loop is visited once.
fn scan_agent_dir(
    dir: &std::path::Path,
    depth: u8,
    visited: &mut std::collections::HashSet<std::path::PathBuf>,
) -> Result<Option<(std::path::PathBuf, String)>, ToolError> {
    if depth > 16 {
        return Err(ToolError::new(ErrorCode::BadSettings, format!("The custom agent folder {} is nested too deeply to check.", dir.display()))
            .with_remediation("Flatten the agent folder or remove the nested files, then try again."));
    }
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(agent_read_error(dir, &e)),
    };
    if let Ok(canonical) = std::fs::canonicalize(dir)
        && !visited.insert(canonical)
    {
        return Ok(None);
    }
    // Scan every entry, including nested folders: skipping one would be a bypass. The check runs
    // once per turn, so a large agents folder costs one scan, not one per draw.
    for entry in entries {
        let entry = entry.map_err(|e| agent_read_error(dir, &e))?;
        let path = entry.path();
        let meta = std::fs::metadata(&path).map_err(|e| agent_read_error(&path, &e))?;
        if meta.is_dir() {
            if let Some(found) = scan_agent_dir(&path, depth + 1, visited)? {
                return Ok(Some(found));
            }
        } else if meta.is_file() && path.extension().and_then(|e| e.to_str()).is_some_and(|e| e.eq_ignore_ascii_case("md")) {
            let text = read_agent_file(&path)?;
            let modes = agent_permission_modes(&text).map_err(|why| {
                ToolError::new(ErrorCode::BadSettings, format!("Couldn't check the custom agent {}: {why}.", path.display()))
                    .with_remediation("Fix or remove the file, then try again.")
            })?;
            if let Some(mode) = modes.into_iter().find(|mode| PERMISSIVE_MODES.iter().any(|m| m.eq_ignore_ascii_case(mode))) {
                return Ok(Some((path, mode)));
            }
        }
    }
    Ok(None)
}

/// The file's first [`AGENT_FILE_CAP`] bytes: enough for the frontmatter, and a huge body can't
/// stall a build. Lossy decoding keeps a character cut at the cap from hiding the frontmatter.
/// `metadata` follows symlinks deliberately, so a linked agent file is checked like any other.
fn read_agent_file(path: &std::path::Path) -> Result<String, ToolError> {
    use std::io::Read as _;
    let meta = std::fs::metadata(path).map_err(|e| agent_read_error(path, &e))?;
    if !meta.is_file() {
        return Ok(String::new());
    }
    let mut bytes = Vec::new();
    std::fs::File::open(path).map_err(|e| agent_read_error(path, &e))?.take(AGENT_FILE_CAP).read_to_end(&mut bytes).map_err(|e| agent_read_error(path, &e))?;
    let text = String::from_utf8_lossy(&bytes);
    if bytes.len() as u64 >= AGENT_FILE_CAP && !frontmatter_closed(&text) {
        return Err(ToolError::new(
            ErrorCode::BadSettings,
            format!("The custom agent {} has no closing `---` for its frontmatter within the first {} KiB.", path.display(), AGENT_FILE_CAP / 1024),
        )
        .with_remediation("Add the closing `---` before the prompt, or remove the file, then try again."));
    }
    Ok(text.into_owned())
}

/// Whether the file has no frontmatter, or its opening frontmatter has a closing `---` in the
/// bytes read.
fn frontmatter_closed(text: &str) -> bool {
    let mut lines = text.lines();
    let Some(first) = lines.next() else { return true };
    if first.trim_start_matches('\u{feff}').trim() != "---" {
        return true;
    }
    lines.any(|line| line.trim() == "---")
}

/// A custom agent file or folder that couldn't be read: fail the check rather than skip it.
fn agent_read_error(path: &std::path::Path, e: &std::io::Error) -> ToolError {
    ToolError::new(ErrorCode::BadSettings, format!("Couldn't check the custom agent {}: {e}.", path.display()))
        .with_remediation("Check that the file and its folders are readable, or remove them, then try again.")
}

/// The top-level `permissionMode`/`permission-mode` values in an agent's YAML frontmatter.
/// Uses `yaml-rust2`'s event iterator (`Parser::next_token`), not the recursive
/// `Parser::load`/`YamlLoader`: depth and event counts are bounded, and invalid YAML, aliases or
/// over-limit input fail closed rather than being skipped.
fn agent_permission_modes(text: &str) -> Result<Vec<String>, String> {
    use yaml_rust2::parser::{Event, Parser};
    let Some(yaml) = agent_frontmatter(text) else { return Ok(Vec::new()) };
    let mut parser = Parser::new_from_str(&yaml);
    let (mut depth, mut events) = (0usize, 0usize);
    let (mut expect_key, mut want_value) = (false, false);
    let mut modes = Vec::new();
    loop {
        events += 1;
        if events > MAX_YAML_EVENTS {
            return Err("its frontmatter has too many entries to check".into());
        }
        let (event, _) = match parser.next_token() {
            Ok(event) => event,
            Err(e) => return lenient_permission_modes(&yaml).ok_or_else(|| format!("its frontmatter isn't valid YAML: {e}")),
        };
        match event {
            Event::StreamStart | Event::DocumentStart | Event::DocumentEnd | Event::Nothing => {}
            Event::StreamEnd => break,
            Event::MappingStart(..) | Event::SequenceStart(..) => {
                depth = depth.saturating_add(1);
                if depth > MAX_YAML_DEPTH {
                    return Err(format!("its frontmatter is nested more than {MAX_YAML_DEPTH} levels deep"));
                }
                if depth == 1 {
                    expect_key = true;
                }
            }
            Event::MappingEnd | Event::SequenceEnd => {
                depth = depth.saturating_sub(1);
                if depth == 1 {
                    expect_key = true;
                    want_value = false;
                }
            }
            Event::Scalar(value, ..) => {
                if depth == 1 {
                    if expect_key {
                        if value == "<<" {
                            return Err("its frontmatter uses a YAML merge key, which this check doesn't resolve".into());
                        }
                        want_value = value == "permissionMode" || value == "permission-mode";
                        expect_key = false;
                    } else {
                        if want_value {
                            let value = value.trim();
                            if !value.is_empty() {
                                modes.push(value.to_string());
                            }
                            want_value = false;
                        }
                        expect_key = true;
                    }
                }
            }
            Event::Alias(_) => return Err("its frontmatter uses a YAML alias, which this check doesn't resolve".into()),
        }
    }
    Ok(modes)
}

/// A fallback for frontmatter that isn't valid YAML: read column-0 `permissionMode` lines and
/// keep only permissive values. `None` keeps the YAML parse error rather than silently allowing
/// the file.
fn lenient_permission_modes(yaml: &str) -> Option<Vec<String>> {
    let mut modes = Vec::new();
    for line in yaml.lines() {
        if line.chars().next().is_some_and(char::is_whitespace) {
            continue;
        }
        for key in ["permissionMode", "permission-mode"] {
            let Some(rest) = line.strip_prefix(key) else { continue };
            let rest = rest.trim_start();
            let Some(value) = rest.strip_prefix(':') else { continue };
            let value = value.trim();
            let value = value.split(" #").next().unwrap_or(value).trim();
            let value = value.trim_matches(['"', '\'']).trim();
            if !value.is_empty() && PERMISSIVE_MODES.iter().any(|m| m.eq_ignore_ascii_case(value)) {
                modes.push(value.to_string());
            }
        }
    }
    (!modes.is_empty()).then_some(modes)
}

/// The YAML between the opening and closing `---` lines, with the same boundary rule as before.
fn agent_frontmatter(text: &str) -> Option<String> {
    let mut lines = text.lines();
    if lines.next()?.trim_start_matches('\u{feff}').trim() != "---" {
        return None;
    }
    let mut yaml = String::new();
    for line in lines {
        if line.trim() == "---" {
            return Some(yaml);
        }
        yaml.push_str(line);
        yaml.push('\n');
    }
    None
}

impl AgentAdapter for ClaudeCode {
    fn ids(&self) -> &'static [&'static str] {
        &["claude-code", "claude"]
    }

    fn answers_questions(&self) -> bool {
        true
    }

    fn prompt_on_stdin(&self) -> bool {
        true
    }

    fn check_tool_server(&self, stop: &CancelToken) -> Result<(), ToolError> {
        cli_can_start(craft_anything_platform::self_cli_path().as_deref(), stop)
    }

    /// A saved extra argument must not override the default mode or bypass permissions: the build
    /// forces `--permission-mode default` so every prompt reaches `core_action_approve`.
    fn check_native_options(&self, options: &BackendOptions) -> Result<(), ToolError> {
        let ends_options = options.args.iter().any(|arg| arg == "--");
        let denies_prompts = options.args.iter().any(|arg| arg == "--permission-prompts=none")
            || options.args.windows(2).any(|pair| pair[0] == "--permission-prompts" && pair[1] == "none");
        let defines_agents = options.args.iter().any(|arg| arg == "--agents" || arg.starts_with("--agents="));
        if ends_options
            || denies_prompts
            || defines_agents
            || options.args.iter().any(|arg| matches!(arg.split('=').next().unwrap_or(arg), "--permission-mode" | "--dangerously-skip-permissions"))
        {
            return Err(ToolError::new(
                ErrorCode::BadSettings,
                "Backend extra arguments must not set --permission-mode, --dangerously-skip-permissions, --permission-prompts none or --agents, or include a bare --: Claude Code builds run in --permission-mode default so actions that need a prompt reach core_action_approve, and inline agent definitions bypass the file mode check.",
            )
            .with_remediation("Remove that argument; put custom helpers in .claude/agents so their modes are checked, and allow what the build needs in the project's approvals instead, then try again."));
        }
        Ok(())
    }

    /// A local Deep build may bring its own Agent/Task; every other run must not pre-add a
    /// helper through saved `--tools`, which would outlive the generated tool list.
    fn check_saved_tools(&self, options: &BackendOptions, allow_helpers: bool) -> Result<(), ToolError> {
        if allow_helpers || !saved_tools_include_agents(&options.args) {
            return Ok(());
        }
        Err(ToolError::new(
            ErrorCode::BadSettings,
            "Backend --tools must not add Agent or Task when this run has no helper agents (a chat, question, Clean room or remote Deep build).",
        )
        .with_remediation("Remove Agent/Task from the saved --tools, or use a local Deep build; put custom helpers in .claude/agents, then try again."))
    }

    /// Refuse a local custom subagent whose own `permissionMode` would accept edits without
    /// asking `core_action_approve`. A remote run's agents are on the other computer and aren't
    /// read here.
    fn check_subagent_modes(&self, config: &BackendConfig, options: &BackendOptions, project: Option<&Path>) -> Result<(), ToolError> {
        if config.ssh_host.as_deref().is_some_and(|h| !h.trim().is_empty()) {
            return Ok(());
        }
        if options.args.iter().any(|arg| arg == "--add-dir" || arg.starts_with("--add-dir=")) {
            return Err(ToolError::new(
                ErrorCode::BadSettings,
                "Backend --add-dir is not supported for a local Deep build: Claude could load custom helper agents from those directories without checking their permissionMode.",
            )
            .with_remediation("Remove --add-dir, or move the helpers into .claude/agents so their modes are checked, then try again."));
        }
        let config_dir = child_path(options.config_dir_here().or_else(|| self.config_dir()), project);
        let Some((path, mode)) = permissive_subagent(config_dir.as_deref(), project)? else { return Ok(()) };
        Err(ToolError::new(
            ErrorCode::BadSettings,
            format!("Claude Code's custom helper agent {} sets permissionMode: {mode}, so its edits would skip this project's approvals.", path.display()),
        )
        .with_remediation(
            "Change that subagent to permissionMode: default, or remove it, then try again. A default helper's edits that need a prompt go through core_action_approve; saved Claude Code allow rules and a helper's PreToolUse hook can still bypass it.",
        ))
    }

    fn model_in_use(&self, options: Option<&BackendOptions>) -> Option<String> {
        Some(options.and_then(|options| options.cli_model.clone()).unwrap_or_else(|| crate::anthropic::DEFAULT_MODEL.into()))
    }

    fn model_choices(&self, _config: &BackendConfig, _options: &BackendOptions) -> Result<Vec<String>, ToolError> {
        Ok(crate::anthropic::MODELS.iter().map(|model| model.to_string()).collect())
    }

    /// Builds Claude Code's headless arguments. A Deep build adds `Agent` to `--tools` so it can
    /// start subagents (#1105); chats, questions and Clean room builds leave it out.
    fn args(&self, cx: &ArgsCx) -> Vec<String> {
        let mut a = cx.options.args.clone();
        if cx.question {
            return question_args(a, cx);
        }
        let mcp_json = json!({"mcpServers": {MCP_NAME: {"command": cx.mcp.command, "args": cx.mcp.args}}}).to_string();
        a.extend(["-p".into(), "--output-format".into(), "stream-json".into(), "--verbose".into(), "--append-system-prompt".into(), cx.preamble.to_string()]);
        // Opus 5.5 everywhere unless config.toml names another model on purpose; the effort
        // goes in through the environment (see `run_cli`).
        a.extend(["--model".into(), cx.options.cli_model.clone().unwrap_or_else(|| crate::anthropic::DEFAULT_MODEL.into())]);
        if let Some(r) = cx.resume {
            a.extend(["--resume".into(), r.to_string()]);
        }
        let tools = match (cx.builder, cx.reads_checked) {
            (true, false) => DEEP_BUILDER_TOOLS,
            (true, true) => BUILDER_TOOLS,
            _ => "",
        };
        a.extend(["--tools".into(), tools.into()]);
        if cx.builder {
            // Manual is Claude Code's default mode; forcing it means a saved acceptEdits, auto or
            // bypassPermissions default can't let a helper skip the approval tool.
            // Questions go through our core_person_ask tool, which the app shows as buttons.
            a.extend([
                "--permission-mode".into(),
                "default".into(),
                "--permission-prompt-tool".into(),
                format!("mcp__{MCP_NAME}__{APPROVAL_TOOL}"),
                "--disallowedTools".into(),
                "AskUserQuestion".into(),
            ]);
        }
        if let Some(dir) = cx.mcp.notes.as_deref().filter(|_| cx.builder) {
            // After every tool call, notes the person added meanwhile reach the model as context.
            a.extend(["--settings".into(), notes_hook_settings(&cx.mcp.command, dir)]);
        }
        a.extend(["--strict-mcp-config".into(), "--mcp-config".into(), mcp_json, "--allowedTools".into(), format!("mcp__{MCP_NAME}__*")]);
        if cx.builder && cx.reads_checked {
            a.push("TodoWrite".into());
        } else if cx.builder {
            a.extend(["Read", "Glob", "Grep", "TodoWrite"].map(String::from));
        } else if cx.reads_checked {
            // A Clean room chat works only through our tools, which keep the original out of reach.
            // Refused outright: neither its defaults nor an allow rule added later can open them (#564).
            a.push("--disallowedTools".into());
            a.extend(READING_TOOLS.iter().chain(CHANGING_TOOLS).map(|t| t.to_string()));
        }
        a
    }

    /// Claude Code's stream-json: text and tool calls as they come, the session id from any line,
    /// and from the final `result` its usage and cost, a failure in plain words, or a session to
    /// resume that isn't on this computer (`session_missing`, with its own message).
    fn parse(&self, p: &mut StreamParser, v: &Value) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        if let Some(id) = v.get("session_id").and_then(Value::as_str) {
            p.session_id = Some(id.to_string());
        }
        match s("type").as_str() {
            "system" if s("subtype") == "init" => {
                let failed = v.get("mcp_servers").and_then(Value::as_array).and_then(|servers| {
                    servers.iter().find(|server| {
                        server.get("name").and_then(Value::as_str) == Some(MCP_NAME)
                            && matches!(server.get("status").and_then(Value::as_str), Some("failed" | "needs-auth"))
                    })
                });
                if let Some(server) = failed {
                    p.failure = Some(TurnFailure::new(FailureKind::ToolsUnavailable));
                    let status = server.get("status").and_then(Value::as_str).unwrap_or_default();
                    out.push(AgentEvent::Error {
                        message: format!("Craft Anything's tools did not start in Claude Code (server status: {status})."),
                        remediation: Some(
                            "Check the Craft Anything CLI and the agent failure log, then start a new build attempt after fixing the tool connection.".into(),
                        ),
                        log: None,
                    });
                }
            }
            "assistant" => {
                if let Some(code) = v.get("error").and_then(Value::as_str) {
                    let failure = TurnFailure::claude(code);
                    p.failure = Some(failure);
                    let why = v.pointer("/message/content").map(content_text).unwrap_or_else(|| code.to_string());
                    let (message, hint) = claude_code_failure_in(&why, self.folder.as_deref());
                    let remediation = hint.or_else(|| match failure.kind {
                        FailureKind::Authentication => self.sign_in_fix(),
                        FailureKind::Billing => Some("Check the account's credits, plan and spending limit before trying again.".into()),
                        FailureKind::Configuration => Some("Check the chosen model and Claude Code settings before trying again.".into()),
                        _ => None,
                    });
                    return vec![AgentEvent::Error { message, remediation, log: None }];
                }
                for b in v.pointer("/message/content").and_then(Value::as_array).cloned().unwrap_or_default() {
                    match b.get("type").and_then(Value::as_str).unwrap_or_default() {
                        "text" => out.push(AgentEvent::TextDelta { text: b.get("text").and_then(Value::as_str).unwrap_or_default().to_string() }),
                        "tool_use" => {
                            let id = b.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
                            let name = b.get("name").and_then(Value::as_str).unwrap_or_default().to_string();
                            out.push(p.started(&id, &name, b.get("input").cloned().unwrap_or(Value::Null)));
                        }
                        _ => {}
                    }
                }
            }
            "user" => {
                for b in v.pointer("/message/content").and_then(Value::as_array).cloned().unwrap_or_default() {
                    if b.get("type").and_then(Value::as_str) == Some("tool_result") {
                        let id = b.get("tool_use_id").and_then(Value::as_str).unwrap_or_default().to_string();
                        let err = b.get("is_error").and_then(Value::as_bool).unwrap_or(false);
                        let text = content_text(b.get("content").unwrap_or(&Value::Null));
                        out.push(p.finished(&id, !err, &text));
                    }
                }
            }
            "result" => {
                p.saw_result = true;
                p.cost_usd = v.get("total_cost_usd").and_then(Value::as_f64);
                if let Some(u) = v.get("usage") {
                    p.input_tokens = u
                        .get("input_tokens")
                        .and_then(Value::as_u64)
                        .unwrap_or(0)
                        .saturating_add(u.get("cache_read_input_tokens").and_then(Value::as_u64).unwrap_or(0));
                    p.output_tokens = u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
                }
                if v.get("is_error").and_then(Value::as_bool).unwrap_or(false) && no_such_session(v) {
                    p.session_missing = true;
                    out.push(AgentEvent::Error {
                        message: "Claude Code's earlier conversation about this project isn't on this computer.".into(),
                        remediation: Some(
                            "It was started on another computer or in another folder. Ask again: a new conversation starts from what the project already has."
                                .into(),
                        ),
                        log: None,
                    });
                } else if v.get("is_error").and_then(Value::as_bool).unwrap_or(false) {
                    p.failure.get_or_insert(TurnFailure::new(FailureKind::Unknown));
                    let why = if s("result").is_empty() { s("subtype") } else { s("result") };
                    let (message, remediation) = claude_code_failure_in(&why, self.folder.as_deref());
                    out.push(AgentEvent::Error { message, remediation, log: None });
                }
            }
            _ => {}
        }
        out
    }

    fn resumes(&self) -> bool {
        true
    }

    /// The person's own standalone Claude Code (see [`host_session_vars`]), in its chosen folder.
    /// Under a Flatpak without one, it gets a folder it can write its conversations to (#506).
    fn process(&self, program: &str) -> Process {
        let proc = standalone(program);
        if let Some(dir) = &self.folder {
            return proc.env("CLAUDE_CONFIG_DIR", dir);
        }
        match flatpak_claude_dir() {
            Some(dir) => {
                let _ = std::fs::create_dir_all(&dir);
                proc.env("CLAUDE_CONFIG_DIR", dir)
            }
            None => proc,
        }
    }

    fn signed_in(&self, program: &str) -> Option<bool> {
        let out = self.process(program).args(["auth", "status", "--json"]).timeout(Duration::from_secs(15)).stdout_cap(64 * 1024).run().ok()?;
        let v: Value = serde_json::from_str(out.stdout_text().trim()).ok()?;
        v.get("loggedIn").and_then(Value::as_bool)
    }

    /// The account its `.claude.json` names (`oauthAccount`), kept there while the sign-in itself
    /// is in the system's keychain. In a chosen folder the file is inside it; with its usual
    /// `~/.claude`, it's beside it in the home folder.
    fn signed_in_before(&self) -> bool {
        let set = self.folder.clone().or_else(|| std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()).map(std::path::PathBuf::from));
        let file = match set.or_else(flatpak_claude_dir) {
            Some(dir) => dir.join(".claude.json"),
            None => match dirs::home_dir() {
                Some(home) => home.join(".claude.json"),
                None => return false,
            },
        };
        claude_names_account(&file)
    }

    fn sign_in_fix(&self) -> Option<String> {
        sign_in_hint_in("login", in_flatpak(), self.folder.as_deref())
    }

    /// The session under Claude Code's own folder (see [`claude_session_file`]).
    fn session_file(&self, id: &str) -> Option<std::path::PathBuf> {
        claude_session_file(&self.config_dir()?, id)
    }

    /// The kept session's requests and replies (see [`claude_handoff`]).
    fn handoff(&self, saved: &str, max: usize) -> Option<String> {
        claude_handoff(saved, max)
    }

    /// Back under Claude Code's own folder (see [`claude_restore`]); without one there's nowhere
    /// to put it, which is said.
    fn restore_session(&self, id: &str, saved: &[u8], cwd: &std::path::Path) -> Result<bool, ToolError> {
        let config = self.config_dir().ok_or_else(|| ToolError::new(ErrorCode::BadSettings, "Couldn't find Claude Code's folder on this computer."))?;
        claude_restore(&config, id, saved, cwd)
    }

    fn folder_var(&self) -> Option<&'static str> {
        Some("CLAUDE_CONFIG_DIR")
    }

    fn in_folder(&self, folder: std::path::PathBuf) -> Option<Arc<dyn AgentAdapter>> {
        Some(Arc::new(ClaudeCode { folder: Some(folder) }))
    }
}

/// Claude Code's own folder: `CLAUDE_CONFIG_DIR` when it's set (the person's choice, which the
/// agent we start keeps too), else our own under a Flatpak (see [`flatpak_claude_dir`]), else
/// `~/.claude`.
fn claude_config_dir() -> Option<std::path::PathBuf> {
    let set = std::env::var_os("CLAUDE_CONFIG_DIR").filter(|v| !v.is_empty()).map(std::path::PathBuf::from);
    set.or_else(flatpak_claude_dir).or_else(|| dirs::home_dir().map(|h| h.join(".claude")))
}

pub use craft_anything_platform::in_flatpak;

/// Where Claude Code keeps its conversations when we start it under a Flatpak and the person
/// hasn't chosen a folder: its own is `~/.claude`, which the sandbox only lets us read, so a
/// conversation was never saved and `--resume` found nothing (#506). A folder of ours in the
/// app's data folder is writable, and kept with the app. `None` outside a Flatpak, or when
/// `CLAUDE_CONFIG_DIR` is set.
pub fn flatpak_claude_dir() -> Option<std::path::PathBuf> {
    let chosen = std::env::var_os("CLAUDE_CONFIG_DIR").is_some_and(|v| !v.is_empty());
    flatpak_claude_dir_in(in_flatpak(), chosen, dirs::data_dir())
}

pub fn flatpak_claude_dir_in(flatpak: bool, chosen: bool, data: Option<std::path::PathBuf>) -> Option<std::path::PathBuf> {
    (flatpak && !chosen).then_some(data?.join("craft-anything").join("claude"))
}

/// Claude Code's name for the folder a session ran in: the path with everything but letters and
/// digits made `-`. It finds a session by its id in any of these folders (2.1), so a long path
/// cut short still works.
fn claude_project_folder(cwd: &std::path::Path) -> String {
    cwd.to_string_lossy().chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '-' }).take(200).collect()
}

/// Put a kept Claude Code session back under `config` (see [`AgentAdapter::restore_session`]).
/// Only a copy of that same session is put back, and never over one that's there.
pub fn claude_restore(config: &std::path::Path, id: &str, saved: &[u8], cwd: &std::path::Path) -> Result<bool, ToolError> {
    let valid = !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-');
    if !valid {
        return Err(ToolError::invalid("The project names its builder's session with an id Claude Code doesn't use."));
    }
    if claude_session_file(config, id).is_some() {
        return Ok(false);
    }
    let named = format!("\"sessionId\":\"{id}\"");
    if !saved.windows(named.len()).any(|w| w == named.as_bytes()) {
        return Err(ToolError::invalid("The conversation kept in the project is of another session."));
    }
    let path = config.join("projects").join(claude_project_folder(cwd)).join(format!("{id}.jsonl"));
    craft_anything_platform::fs::write_atomic_private(&path, saved)?;
    Ok(true)
}

/// Folders under Claude Code's `projects/` looked through for a session.
const SESSION_FOLDERS: usize = 10_000;

/// Claude Code keeps each session as `projects/<the folder it ran in>/<id>.jsonl` under its
/// config folder. An id is a UUID: anything else (a hand-edited manifest) names no file.
pub fn claude_session_file(config: &std::path::Path, id: &str) -> Option<std::path::PathBuf> {
    if id.is_empty() || id.len() > 64 || !id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-') {
        return None;
    }
    let name = format!("{id}.jsonl");
    std::fs::read_dir(config.join("projects")).ok()?.flatten().take(SESSION_FOLDERS).map(|e| e.path().join(&name)).find(|p| p.is_file())
}

/// Whether Claude Code's `.claude.json` names the account it was signed in to (`oauthAccount`).
/// It also keeps every project's history, so up to 32 MB is read; a bigger or unreadable file
/// names none.
pub fn claude_names_account(file: &std::path::Path) -> bool {
    use std::io::Read;
    const CAP: u64 = 32 << 20;
    let mut text = Vec::new();
    let read = std::fs::File::open(file).and_then(|f| f.take(CAP + 1).read_to_end(&mut text));
    if read.is_err() || text.len() as u64 > CAP {
        return false;
    }
    serde_json::from_slice::<Value>(&text).ok().is_some_and(|v| v.get("oauthAccount").is_some_and(Value::is_object))
}

/// Longest single request or reply in a handoff; a first brief runs to many thousands.
const HANDOFF_ENTRY: usize = 1500;

/// A Claude Code session read back as a handoff: the messages Craft Anything sent and the text
/// the builder answered with, oldest first, leaving out tool calls, their results, thinking and
/// side tasks. The newest are kept when it's longer than `max` characters.
pub fn claude_handoff(saved: &str, max: usize) -> Option<String> {
    let clip = |t: &str| {
        let t = t.trim();
        if t.chars().count() > HANDOFF_ENTRY { format!("{}…", t.chars().take(HANDOFF_ENTRY).collect::<String>()) } else { t.to_string() }
    };
    let mut entries = Vec::new();
    for v in saved.lines().filter_map(|l| serde_json::from_str::<Value>(l).ok()) {
        if v.get("isSidechain").and_then(Value::as_bool) == Some(true) {
            continue;
        }
        let content = v.pointer("/message/content");
        match v.get("type").and_then(Value::as_str) {
            Some("user") => {
                if let Some(t) = content.and_then(Value::as_str).filter(|t| !t.trim().is_empty()) {
                    entries.push(format!("Craft Anything asked you: {}", clip(t)));
                }
            }
            Some("assistant") => {
                let text: String = content
                    .and_then(Value::as_array)
                    .into_iter()
                    .flatten()
                    .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
                    .filter_map(|b| b.get("text").and_then(Value::as_str))
                    .collect::<Vec<_>>()
                    .join("\n");
                if !text.trim().is_empty() && text.trim() != "No response requested." {
                    entries.push(format!("You answered: {}", clip(&text)));
                }
            }
            _ => {}
        }
    }
    let mut kept = Vec::new();
    let mut used = 0usize;
    for e in entries.iter().rev() {
        let n = e.chars().count().saturating_add(2);
        if used.saturating_add(n) > max {
            break;
        }
        used = used.saturating_add(n);
        kept.push(e.as_str());
    }
    kept.reverse();
    (!kept.is_empty()).then(|| kept.join("\n\n"))
}

/// Codex (`codex exec --json`). Flags are checked against `codex exec --help` before use.
pub struct Codex;

impl AgentAdapter for Codex {
    fn ids(&self) -> &'static [&'static str] {
        &["codex"]
    }

    /// `codex exec -` reads the message from stdin, so a long one can't pass Windows'
    /// command-line limit (os error 206, #528).
    fn prompt_on_stdin(&self) -> bool {
        true
    }

    fn stdin_text(&self, preamble: &str, prompt: &str) -> String {
        format!("{preamble}{prompt}")
    }

    fn check_tool_server(&self, stop: &CancelToken) -> Result<(), ToolError> {
        cli_can_start(craft_anything_platform::self_cli_path().as_deref(), stop)
    }

    fn args(&self, cx: &ArgsCx) -> Vec<String> {
        let mut v = vec!["exec".to_string(), "--json".into()];
        // Project folders aren't git repositories, and `codex exec` refuses to run outside one
        // without this (#69). Not twice: it may already be in the backend's own args.
        const SKIP_GIT: &str = "--skip-git-repo-check";
        if !cx.options.args.iter().any(|a| a == SKIP_GIT) {
            v.push(SKIP_GIT.into());
        }
        if cx.builder {
            v.extend(["--sandbox".into(), "workspace-write".into()]);
        }
        v.extend(["-c".into(), format!("mcp_servers.{MCP_NAME}.command={}", toml_string(&cx.mcp.command))]);
        // Codex doesn't refresh its callable tools after enable_toolset (#538), so list every
        // permitted tool from startup. The server still applies Clean room's restrictions.
        let mut mcp_args = cx.mcp.args.clone();
        if !mcp_args.iter().any(|a| a == "--all-tools") {
            mcp_args.push("--all-tools".into());
        }
        let args_toml = format!("[{}]", mcp_args.iter().map(|s| toml_string(s)).collect::<Vec<_>>().join(", "));
        v.extend(["-c".into(), format!("mcp_servers.{MCP_NAME}.args={args_toml}")]);
        // `codex exec` can't ask, so without this it refuses a tool not marked read-only, such as
        // rebuild_start, before our server sees the call (#540). Only tools that ask the person
        // themselves are approved; every other tool keeps the person's own Codex policy.
        for tool in craft_anything_engine::AGENT_APP_PREAPPROVED_TOOLS {
            v.extend(["-c".into(), format!("mcp_servers.{MCP_NAME}.tools.{tool}.approval_mode=\"approve\"")]);
        }
        if let Some(m) = &cx.options.cli_model {
            v.extend(["-m".into(), m.clone()]);
        }
        v.extend(cx.options.args.iter().cloned());
        // The message itself goes in on stdin (see `stdin_text`).
        v.push("-".into());
        v
    }

    fn required_flags(&self) -> (&'static [&'static str], &'static [&'static str]) {
        (&["exec", "--help"], &["--json"])
    }

    /// Plain `codex <prompt>` is the interactive TUI: "stdin is not a terminal".
    fn plain_fallback(&self) -> bool {
        false
    }

    fn parse(&self, p: &mut StreamParser, v: &Value) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        let t = v.get("type").and_then(Value::as_str).unwrap_or_default();
        if let Some(id) = v.get("thread_id").and_then(Value::as_str) {
            p.session_id = Some(id.to_string());
        }
        let item = v.get("item").cloned().unwrap_or(Value::Null);
        let it = item.get("type").and_then(Value::as_str).unwrap_or_default();
        let id = item.get("id").and_then(Value::as_str).unwrap_or_default().to_string();
        match (t, it) {
            ("item.completed", "agent_message") => {
                out.push(AgentEvent::TextDelta { text: format!("{}\n", item.get("text").and_then(Value::as_str).unwrap_or_default()) })
            }
            ("item.completed", "reasoning") => out.push(AgentEvent::Status { text: summary_of(item.get("text").and_then(Value::as_str).unwrap_or_default()) }),
            ("item.started", "mcp_tool_call") => {
                let name = item.get("tool").and_then(Value::as_str).unwrap_or_default().to_string();
                out.push(p.started(&id, &name, item.get("arguments").cloned().unwrap_or(Value::Null)));
            }
            ("item.completed", "mcp_tool_call") => {
                if !p.tool_names.contains_key(&id) {
                    let name = item.get("tool").and_then(Value::as_str).unwrap_or_default().to_string();
                    out.push(p.started(&id, &name, item.get("arguments").cloned().unwrap_or(Value::Null)));
                }
                let failed = item.get("status").and_then(Value::as_str) == Some("failed") || item.get("error").is_some_and(|e| !e.is_null());
                let tool = item.get("tool").and_then(Value::as_str).unwrap_or_default();
                let ours = item.get("server").and_then(Value::as_str) == Some(MCP_NAME);
                let text = codex_tool_text(&item, tool, ours);
                out.push(p.finished(&id, !failed, &text));
            }
            ("item.started", "command_execution") => {
                out.push(AgentEvent::Status { text: format!("Running: {}", summary_of(item.get("command").and_then(Value::as_str).unwrap_or_default())) })
            }
            ("turn.completed", _) => {
                p.saw_result = true;
                if let Some(u) = v.get("usage") {
                    p.input_tokens = p.input_tokens.saturating_add(u.get("input_tokens").and_then(Value::as_u64).unwrap_or(0));
                    p.output_tokens = p.output_tokens.saturating_add(u.get("output_tokens").and_then(Value::as_u64).unwrap_or(0));
                }
            }
            ("turn.failed", _) | ("error", _) => {
                let m = v.pointer("/error/message").or_else(|| v.get("message")).and_then(Value::as_str).unwrap_or("unknown error");
                out.push(AgentEvent::Error { message: format!("Codex: {m}"), remediation: None, log: None });
            }
            _ => {}
        }
        out
    }
}

/// What a finished Codex tool call says. A failed call carries `"result": null` beside its
/// `error`, which showed as "null" (#540), so null fields are skipped and an error's message is
/// used as written. Codex's refusal to run one of our tools it can't ask about gets the way out.
fn codex_tool_text(item: &Value, tool: &str, ours: bool) -> String {
    let field = |k: &str| item.get(k).filter(|v| !v.is_null());
    let error = field("error").map(|e| e.get("message").and_then(Value::as_str).map_or_else(|| e.to_string(), str::to_string));
    let text = match (field("result"), error) {
        (_, Some(e)) => e,
        (Some(r), None) => r.to_string(),
        (None, None) => String::new(),
    };
    // Another MCP server's refusal isn't fixed by our setup: it keeps Codex's own words.
    if ours && text.contains(CODEX_NEEDS_APPROVAL) {
        // The feed keeps 200 characters of a result, so the fix comes first and fits.
        return format!(
            "Codex blocked {tool}: it can't ask for approval here. Fix: run `craft-anything-cli setup --client codex --yes`, then retry. See docs/codex.md."
        );
    }
    text
}

/// Codex's wording when a tool needs approval and its policy is `never` (#540).
const CODEX_NEEDS_APPROVAL: &str = "requires approval, but approval policy is never";

/// Gemini CLI (`--output-format stream-json`). Flags are checked against `gemini --help`.
pub struct GeminiCli;

impl AgentAdapter for GeminiCli {
    fn ids(&self) -> &'static [&'static str] {
        &["gemini-cli", "gemini"]
    }

    /// Gemini CLI reads its message from stdin and appends `-p` to it, so a long one can't pass
    /// Windows' command-line limit, which is 8,191 characters through npm's `gemini.cmd`.
    fn prompt_on_stdin(&self) -> bool {
        true
    }

    fn stdin_text(&self, preamble: &str, prompt: &str) -> String {
        format!("{preamble}{prompt}")
    }

    fn args(&self, cx: &ArgsCx) -> Vec<String> {
        let mut a = cx.options.args.clone();
        if let Some(m) = &cx.options.cli_model {
            a.extend(["-m".into(), m.clone()]);
        }
        // An empty `-p` keeps it headless and adds nothing to the message on stdin (see `stdin_text`).
        a.extend(["--output-format".into(), "stream-json".into(), "-p".into(), String::new()]);
        a
    }

    fn required_flags(&self) -> (&'static [&'static str], &'static [&'static str]) {
        (&["--help"], &["--output-format", "stream-json"])
    }

    fn parse(&self, p: &mut StreamParser, v: &Value) -> Vec<AgentEvent> {
        let mut out = Vec::new();
        let s = |k: &str| v.get(k).and_then(Value::as_str).unwrap_or_default().to_string();
        match s("type").as_str() {
            "init" => p.session_id = v.get("session_id").and_then(Value::as_str).map(str::to_string),
            "message" if s("role") == "assistant" => out.push(AgentEvent::TextDelta { text: s("content") }),
            "tool_use" => {
                let id = s("tool_id");
                let name = s("tool_name");
                out.push(p.started(&id, &name, v.get("parameters").cloned().unwrap_or(Value::Null)));
            }
            "tool_result" => {
                let id = s("tool_id");
                let ok = s("status") != "error";
                let text = v.get("output").map(content_text).unwrap_or_default();
                out.push(p.finished(&id, ok, &text));
            }
            "error" => out.push(AgentEvent::Error { message: format!("Gemini CLI: {}", s("message")), remediation: None, log: None }),
            "result" => {
                p.saw_result = true;
                if let Some(st) = v.get("stats") {
                    p.input_tokens = st.get("input_tokens").and_then(Value::as_u64).unwrap_or(0);
                    p.output_tokens = st.get("output_tokens").and_then(Value::as_u64).unwrap_or(0);
                }
            }
            _ => {}
        }
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn managed_agents_directory_is_scanned() {
        let expected = if cfg!(target_os = "macos") {
            std::path::PathBuf::from("/Library/Application Support/ClaudeCode")
        } else if cfg!(target_os = "windows") {
            std::path::PathBuf::from(r"C:\Program Files\ClaudeCode")
        } else {
            std::path::PathBuf::from("/etc/claude-code")
        };
        assert!(agent_dirs(None, None).contains(&expected.join(".claude").join("agents")));
    }
}
