//! Email support for golem test flows that need to receive a verification
//! code or notification sent to a disposable inbox (e.g. an OTP during a
//! sign-up flow).
//!
//! Two pieces cover the round trip: [`EtherealClient`] provisions a
//! throwaway [`EtherealAccount`] from the Ethereal/Nodemailer test-SMTP
//! service, and [`ImapPoller`] then polls that account's IMAP inbox via
//! [`ImapPoller::await_email`] until a message with a matching subject
//! arrives, returning it as a parsed [`EmailMessage`]. Both the HTTP call
//! ([`HttpClient`]) and the IMAP session ([`ImapConnection`]) are behind
//! injectable traits so the crate's tests never hit the network. Consumers
//! such as `golem-runner`'s external actions wire the real backends
//! together to implement `await_email`-style test steps.
#![deny(clippy::unwrap_used)]

mod ethereal;
mod imap_poller;

pub use ethereal::{EtherealAccount, EtherealClient, HttpClient};
pub use imap_poller::{EmailMessage, ImapConnection, ImapPoller};
