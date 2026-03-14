// golem-email: email notification support for GOLEM test framework
#![deny(clippy::unwrap_used)]

mod ethereal;
mod imap_poller;

pub use ethereal::{EtherealAccount, EtherealClient, HttpClient};
pub use imap_poller::{EmailMessage, ImapConnection, ImapPoller};
