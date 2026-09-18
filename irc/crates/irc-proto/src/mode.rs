//! Mode-change parsing.
//!
//! The mode string in a `MODE` command is a sequence of `+`/`-` toggles
//! followed by one-byte mode letters. Separately, a list of arguments
//! accompanies the change, consumed in order by whichever modes the
//! advertised [`ModeSpec`] says take an argument.
//!
//! Example (channel):
//!
//! ```text
//! MODE #rust +o-b+l alice *!*@spam.example 50
//! ```
//!
//! With an `rfc2812` spec this parses into:
//!
//! - `+o alice`
//! - `-b *!*@spam.example`
//! - `+l 50`

use bytes::Bytes;

use crate::isupport::Isupport;

/// Describes which mode letters take arguments.
///
/// Populated from ISUPPORT `CHANMODES=` and `PREFIX=` or defaulted to
/// the RFC 2812 canonical set via [`ModeSpec::rfc2812`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeSpec {
    /// Class A: list modes (always take an arg — e.g. `+b` bans).
    pub list_modes: Bytes,
    /// Class B: setting and unsetting both take an arg (e.g. `+k` key).
    pub setunset_modes: Bytes,
    /// Class C: setting takes an arg, unsetting does not (e.g. `+l` limit).
    pub set_only_modes: Bytes,
    /// Class D: flag modes with no arg (e.g. `+t`, `+n`).
    pub flag_modes: Bytes,
    /// Prefix modes from `PREFIX=(modes)symbols` — always take a nick arg.
    pub prefix_modes: Bytes,
}

impl Default for ModeSpec {
    fn default() -> Self {
        Self::rfc2812()
    }
}

impl ModeSpec {
    /// The RFC 2812 canonical mode taxonomy, with the PREFIX (`ov`)
    /// modes recognised.
    #[must_use]
    pub fn rfc2812() -> Self {
        Self {
            list_modes: Bytes::from_static(b"beI"),
            setunset_modes: Bytes::from_static(b"k"),
            set_only_modes: Bytes::from_static(b"l"),
            flag_modes: Bytes::from_static(b"imnpstr"),
            prefix_modes: Bytes::from_static(b"ov"),
        }
    }

    /// Build a spec from a server's ISUPPORT set.
    ///
    /// Reads `CHANMODES=A,B,C,D` and `PREFIX=(modes)symbols`. Falls
    /// back to the [`ModeSpec::rfc2812`] defaults for any token that
    /// cannot be parsed.
    #[must_use]
    pub fn from_isupport(isupport: &Isupport) -> Self {
        let mut spec = Self::rfc2812();
        if let Some(tok) = isupport.get(b"CHANMODES") {
            if let Some(value) = &tok.value {
                let mut parts = value.as_ref().split(|b| *b == b',');
                if let Some(a) = parts.next() {
                    spec.list_modes = Bytes::copy_from_slice(a);
                }
                if let Some(b) = parts.next() {
                    spec.setunset_modes = Bytes::copy_from_slice(b);
                }
                if let Some(c) = parts.next() {
                    spec.set_only_modes = Bytes::copy_from_slice(c);
                }
                if let Some(d) = parts.next() {
                    spec.flag_modes = Bytes::copy_from_slice(d);
                }
            }
        }
        if let Some(tok) = isupport.get(b"PREFIX") {
            if let Some(value) = &tok.value {
                // Form `(modes)symbols` — extract bytes inside parens.
                let v: &[u8] = value.as_ref();
                if v.first() == Some(&b'(') {
                    if let Some(close) = v.iter().position(|b| *b == b')') {
                        spec.prefix_modes = Bytes::copy_from_slice(&v[1..close]);
                    }
                }
            }
        }
        spec
    }

    fn arg_count(&self, letter: u8, adding: bool) -> usize {
        if self.prefix_modes.contains(&letter) || self.list_modes.contains(&letter) {
            return 1;
        }
        if self.setunset_modes.contains(&letter) {
            return 1;
        }
        if self.set_only_modes.contains(&letter) {
            return usize::from(adding);
        }
        0
    }
}

/// A single parsed mode toggle.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModeChange {
    /// `true` for `+x`, `false` for `-x`.
    pub adding: bool,
    /// Mode letter byte.
    pub letter: u8,
    /// Argument if the letter takes one under the active [`ModeSpec`].
    pub arg: Option<Bytes>,
}

/// Parse a channel mode change string with its accompanying args.
///
/// Unknown mode letters are treated as class-D flag modes (no arg).
/// Leading toggles without a prior `+`/`-` default to adding.
#[must_use]
pub fn parse_channel_modes(changes: &[u8], args: &[Bytes], spec: &ModeSpec) -> Vec<ModeChange> {
    let mut out = Vec::new();
    let mut adding = true;
    let mut arg_idx = 0;
    for &b in changes {
        match b {
            b'+' => adding = true,
            b'-' => adding = false,
            letter => {
                let take = spec.arg_count(letter, adding);
                let arg = if take == 1 && arg_idx < args.len() {
                    let a = args[arg_idx].clone();
                    arg_idx += 1;
                    Some(a)
                } else {
                    None
                };
                out.push(ModeChange {
                    adding,
                    letter,
                    arg,
                });
            }
        }
    }
    out
}

/// Parse a user mode change string. User modes never take args.
#[must_use]
pub fn parse_user_modes(changes: &[u8]) -> Vec<ModeChange> {
    let mut out = Vec::new();
    let mut adding = true;
    for &b in changes {
        match b {
            b'+' => adding = true,
            b'-' => adding = false,
            letter => out.push(ModeChange {
                adding,
                letter,
                arg: None,
            }),
        }
    }
    out
}

/// Render a slice of [`ModeChange`]s back to a `(changes, args)` pair.
///
/// Consecutive toggles of the same direction are grouped (e.g. two
/// additions back-to-back share a single `+`) to match typical server
/// output.
#[must_use]
pub fn write_modes(changes: &[ModeChange]) -> (Bytes, Vec<Bytes>) {
    let mut changes_out = bytes::BytesMut::new();
    let mut args = Vec::new();
    let mut last_dir: Option<bool> = None;
    for ch in changes {
        if last_dir != Some(ch.adding) {
            changes_out.extend_from_slice(if ch.adding { b"+" } else { b"-" });
            last_dir = Some(ch.adding);
        }
        changes_out.extend_from_slice(&[ch.letter]);
        if let Some(arg) = &ch.arg {
            args.push(arg.clone());
        }
    }
    (changes_out.freeze(), args)
}

#[cfg(test)]
mod tests {
    use super::{ModeChange, ModeSpec, parse_channel_modes, parse_user_modes, write_modes};
    use crate::isupport::{Isupport, IsupportToken};
    use bytes::Bytes;

    #[test]
    fn rfc2812_defaults_classify_common_modes() {
        let spec = ModeSpec::rfc2812();
        assert_eq!(spec.arg_count(b'b', true), 1); // list
        assert_eq!(spec.arg_count(b'b', false), 1);
        assert_eq!(spec.arg_count(b'k', true), 1); // set+unset
        assert_eq!(spec.arg_count(b'k', false), 1);
        assert_eq!(spec.arg_count(b'l', true), 1); // set only
        assert_eq!(spec.arg_count(b'l', false), 0);
        assert_eq!(spec.arg_count(b't', true), 0); // flag
        assert_eq!(spec.arg_count(b'o', true), 1); // prefix
    }

    #[test]
    fn channel_mode_with_mixed_classes() {
        let spec = ModeSpec::rfc2812();
        let changes = parse_channel_modes(
            b"+o-b+l",
            &[
                Bytes::from_static(b"alice"),
                Bytes::from_static(b"*!*@spam.example"),
                Bytes::from_static(b"50"),
            ],
            &spec,
        );
        assert_eq!(changes.len(), 3);
        assert_eq!(
            changes[0],
            ModeChange {
                adding: true,
                letter: b'o',
                arg: Some(Bytes::from_static(b"alice")),
            }
        );
        assert!(!changes[1].adding);
        assert_eq!(changes[1].letter, b'b');
        assert_eq!(changes[2].letter, b'l');
        assert_eq!(changes[2].arg.as_deref(), Some(&b"50"[..]));
    }

    #[test]
    fn unsetting_set_only_mode_consumes_no_arg() {
        let spec = ModeSpec::rfc2812();
        let changes = parse_channel_modes(b"-l+t", &[], &spec);
        assert_eq!(changes.len(), 2);
        assert!(!changes[0].adding);
        assert_eq!(changes[0].letter, b'l');
        assert!(changes[0].arg.is_none());
    }

    #[test]
    fn leading_letter_defaults_to_adding() {
        let spec = ModeSpec::rfc2812();
        let changes = parse_channel_modes(b"tn", &[], &spec);
        assert_eq!(changes.len(), 2);
        assert!(changes.iter().all(|c| c.adding));
    }

    #[test]
    fn user_modes_never_take_args() {
        let changes = parse_user_modes(b"+iw-o");
        assert_eq!(changes.len(), 3);
        assert_eq!(changes[0].letter, b'i');
        assert_eq!(changes[2].letter, b'o');
        assert!(!changes[2].adding);
        assert!(changes.iter().all(|c| c.arg.is_none()));
    }

    #[test]
    fn isupport_chanmodes_drives_spec() {
        let mut is = Isupport::new();
        is.merge([
            IsupportToken::parse(&Bytes::from_static(b"CHANMODES=beI,kf,l,imnpstr")),
            IsupportToken::parse(&Bytes::from_static(b"PREFIX=(ohv)@%+")),
        ]);
        let spec = ModeSpec::from_isupport(&is);
        assert_eq!(spec.list_modes.as_ref(), b"beI");
        assert_eq!(spec.setunset_modes.as_ref(), b"kf");
        assert_eq!(spec.set_only_modes.as_ref(), b"l");
        assert_eq!(spec.flag_modes.as_ref(), b"imnpstr");
        assert_eq!(spec.prefix_modes.as_ref(), b"ohv");
        // 'h' now prefix-mode, takes nick arg
        assert_eq!(spec.arg_count(b'h', true), 1);
    }

    #[test]
    fn round_trip_merges_consecutive_directions() {
        let spec = ModeSpec::rfc2812();
        let changes = parse_channel_modes(
            b"+o+v-b",
            &[
                Bytes::from_static(b"alice"),
                Bytes::from_static(b"bob"),
                Bytes::from_static(b"*!*@host"),
            ],
            &spec,
        );
        let (written, args) = write_modes(&changes);
        assert_eq!(written.as_ref(), b"+ov-b");
        assert_eq!(args.len(), 3);
    }
}
