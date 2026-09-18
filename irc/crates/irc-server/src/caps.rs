//! IRCv3 capability advertisement and per-connection tracking.
//!
//! The server advertises a fixed set of capabilities during `CAP LS`.
//! Each connection independently enables capabilities via `CAP REQ`;
//! the [`EnabledCaps`] bitbag tracks which are active.

/// Capabilities this server advertises.
pub const ADVERTISED_CAPS: &[&str] = &[
    "server-time",
    "echo-message",
    "account-notify",
    "away-notify",
    "extended-join",
    "message-tags",
    "multi-prefix",
    "sasl",
    "cap-notify",
];

/// Value string returned for `sasl` in CAP LS 302 (mechanisms we support).
pub const SASL_CAP_VALUE: &str = "PLAIN,EXTERNAL";

/// Per-connection enabled capability set.
#[derive(Debug, Default, Clone)]
#[allow(clippy::struct_excessive_bools)]
pub struct EnabledCaps {
    /// `server-time` — timestamps on messages.
    pub server_time: bool,
    /// `echo-message` — sender receives its own channel messages.
    pub echo_message: bool,
    /// `account-notify` — ACCOUNT changes relayed to peers.
    pub account_notify: bool,
    /// `away-notify` — AWAY changes relayed to peers.
    pub away_notify: bool,
    /// `extended-join` — JOINs carry account + realname.
    pub extended_join: bool,
    /// `message-tags` — arbitrary message tags.
    pub message_tags: bool,
    /// `multi-prefix` — NAMES shows all prefix modes.
    pub multi_prefix: bool,
    /// `sasl` — SASL authentication.
    pub sasl: bool,
    /// `cap-notify` — server MAY push CAP NEW/DEL.
    pub cap_notify: bool,
}

impl EnabledCaps {
    /// Enable a capability by name. Returns `true` if the name is
    /// recognised (and was toggled on).
    pub fn enable(&mut self, name: &str) -> bool {
        match name {
            "server-time" => self.server_time = true,
            "echo-message" => self.echo_message = true,
            "account-notify" => self.account_notify = true,
            "away-notify" => self.away_notify = true,
            "extended-join" => self.extended_join = true,
            "message-tags" => self.message_tags = true,
            "multi-prefix" => self.multi_prefix = true,
            "sasl" => self.sasl = true,
            "cap-notify" => self.cap_notify = true,
            _ => return false,
        }
        true
    }

    /// Disable a capability by name. Returns `true` if the name is
    /// recognised (and was toggled off).
    pub fn disable(&mut self, name: &str) -> bool {
        match name {
            "server-time" => self.server_time = false,
            "echo-message" => self.echo_message = false,
            "account-notify" => self.account_notify = false,
            "away-notify" => self.away_notify = false,
            "extended-join" => self.extended_join = false,
            "message-tags" => self.message_tags = false,
            "multi-prefix" => self.multi_prefix = false,
            "sasl" => self.sasl = false,
            "cap-notify" => self.cap_notify = false,
            _ => return false,
        }
        true
    }

    /// Names of currently enabled capabilities.
    pub fn enabled_names(&self) -> Vec<&'static str> {
        let mut out = Vec::new();
        if self.server_time {
            out.push("server-time");
        }
        if self.echo_message {
            out.push("echo-message");
        }
        if self.account_notify {
            out.push("account-notify");
        }
        if self.away_notify {
            out.push("away-notify");
        }
        if self.extended_join {
            out.push("extended-join");
        }
        if self.message_tags {
            out.push("message-tags");
        }
        if self.multi_prefix {
            out.push("multi-prefix");
        }
        if self.sasl {
            out.push("sasl");
        }
        if self.cap_notify {
            out.push("cap-notify");
        }
        out
    }
}

/// Check whether every name in a space-separated cap list is advertised.
pub fn all_known(names: &[&str]) -> bool {
    names.iter().all(|n| {
        let bare = n.strip_prefix('-').unwrap_or(n);
        ADVERTISED_CAPS.contains(&bare)
    })
}

#[cfg(test)]
mod tests {
    use super::EnabledCaps;

    #[test]
    fn enable_known_cap() {
        let mut caps = EnabledCaps::default();
        assert!(caps.enable("server-time"));
        assert!(caps.server_time);
        assert!(caps.enable("echo-message"));
        assert!(caps.echo_message);
    }

    #[test]
    fn enable_unknown_returns_false() {
        let mut caps = EnabledCaps::default();
        assert!(!caps.enable("batch"));
        assert!(!caps.enable(""));
    }

    #[test]
    fn disable_round_trips() {
        let mut caps = EnabledCaps::default();
        caps.enable("sasl");
        assert!(caps.sasl);
        assert!(caps.disable("sasl"));
        assert!(!caps.sasl);
    }

    #[test]
    fn enabled_names_reflects_state() {
        let mut caps = EnabledCaps::default();
        caps.enable("server-time");
        caps.enable("cap-notify");
        let names = caps.enabled_names();
        assert!(names.contains(&"server-time"));
        assert!(names.contains(&"cap-notify"));
        assert!(!names.contains(&"sasl"));
    }
}
