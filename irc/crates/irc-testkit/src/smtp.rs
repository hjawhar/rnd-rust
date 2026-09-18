//! Outbound email transport abstraction.
//!
//! Production wires this to `lettre`. Tests use [`SmtpSink`] to capture
//! every send for later assertion (e.g. recovering a verification token).

use std::error::Error;
use std::sync::Mutex;

/// A pending outbound email.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutgoingEmail {
    /// Recipient address (RFC 5321 `RCPT TO`).
    pub to: String,
    /// `From:` header.
    pub from: String,
    /// `Subject:` header.
    pub subject: String,
    /// Plain-text body.
    pub body: String,
}

/// Errors returned by an [`SmtpTransport`].
pub type SmtpError = Box<dyn Error + Send + Sync>;

/// SMTP transport contract.
///
/// The shape is intentionally synchronous in Phase 0; an async variant
/// arrives in Phase 3 alongside the real `lettre` integration.
pub trait SmtpTransport: Send + Sync + 'static {
    /// Send a single email.
    fn send(&self, email: OutgoingEmail) -> Result<(), SmtpError>;
}

/// In-memory transport that records every email instead of delivering it.
///
/// Useful in tests to recover verification tokens or assert that the
/// server tried to mail a given address.
#[derive(Debug, Default)]
pub struct SmtpSink {
    inbox: Mutex<Vec<OutgoingEmail>>,
}

impl SmtpSink {
    /// Construct an empty sink.
    #[must_use]
    pub fn new() -> Self {
        Self::default()
    }

    /// Snapshot every captured email.
    ///
    /// # Panics
    /// Panics if the internal lock is poisoned.
    #[must_use]
    pub fn captured(&self) -> Vec<OutgoingEmail> {
        self.inbox.lock().expect("SmtpSink mutex poisoned").clone()
    }

    /// Return the most recent email sent to `address`, if any.
    ///
    /// # Panics
    /// Panics if the internal lock is poisoned.
    #[must_use]
    pub fn last_to(&self, address: &str) -> Option<OutgoingEmail> {
        self.inbox
            .lock()
            .expect("SmtpSink mutex poisoned")
            .iter()
            .rev()
            .find(|m| m.to == address)
            .cloned()
    }
}

impl SmtpTransport for SmtpSink {
    fn send(&self, email: OutgoingEmail) -> Result<(), SmtpError> {
        self.inbox
            .lock()
            .expect("SmtpSink mutex poisoned")
            .push(email);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::{OutgoingEmail, SmtpSink, SmtpTransport};

    fn sample(to: &str) -> OutgoingEmail {
        OutgoingEmail {
            to: to.into(),
            from: "noreply@example.net".into(),
            subject: "Verify".into(),
            body: "token=abc".into(),
        }
    }

    #[test]
    fn sink_captures_each_send_in_order() {
        let sink = SmtpSink::new();
        sink.send(sample("alice@example.com")).unwrap();
        sink.send(sample("bob@example.com")).unwrap();
        let captured = sink.captured();
        assert_eq!(captured.len(), 2);
        assert_eq!(captured[0].to, "alice@example.com");
        assert_eq!(captured[1].to, "bob@example.com");
    }

    #[test]
    fn last_to_returns_most_recent_match() {
        let sink = SmtpSink::new();
        sink.send(sample("alice@example.com")).unwrap();
        let mut second = sample("alice@example.com");
        second.body = "token=xyz".into();
        sink.send(second.clone()).unwrap();
        assert_eq!(sink.last_to("alice@example.com"), Some(second));
        assert!(sink.last_to("nobody@example.com").is_none());
    }
}
