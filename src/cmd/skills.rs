//! `skills`: print the agent guide, `skills/agent-mobile/SKILL.md`.

/// The committed guide; its command list is checked against the registry.
pub const SKILL: &str = include_str!("../../skills/agent-mobile/SKILL.md");

/// The `skills` verb: the guide, no wire involved.
///
/// # Errors
/// Never; the signature matches every local verb.
#[allow(
    clippy::unnecessary_wraps,
    reason = "every local verb shares one fn signature"
)]
pub fn exec(_call: &super::Call) -> Result<super::Reply, agent_mobile_core::error::Failure> {
    Ok(super::Reply::Text(SKILL.trim_end().to_owned()))
}
