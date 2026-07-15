//! Live end-to-end check of the real receive path (tokio-rustls + async-imap)
//! against a freshly provisioned Ethereal account.
//!
//! Ignored by default — it needs the network and provisions a throwaway
//! account, so it never runs in the normal `cargo t` gate. Run it explicitly:
//!
//! ```sh
//! cargo test -p golem-email --test live_ethereal -- --ignored --nocapture
//! ```
//!
//! Unlike the mock-backed unit tests, this drives a real
//! send → connect → login → EXAMINE → FETCH → parse round trip: it SMTP-sends a
//! message to the account (via `lettre`) and then receives it through
//! `async-imap`, so it is the gate that proves the `imap` → `async-imap`
//! migration works against a real IMAP server.

use golem_email::{EtherealClient, ImapPoller};
use lettre::message::header::ContentType;
use lettre::transport::smtp::authentication::Credentials;
use lettre::{AsyncSmtpTransport, AsyncTransport, Message, Tokio1Executor};

#[tokio::test]
#[ignore = "live network: provisions an Ethereal account, sends, then receives"]
async fn live_ethereal_receive() {
    // Provision a fresh disposable account via the Nodemailer API (no creds).
    let account = EtherealClient::new()
        .create_account()
        .await
        .expect("SHALL provision an Ethereal account");

    // A subject unique to this run so the poll can't match a stale message.
    let subject = format!("golem-async-imap-live-{}", std::process::id());

    // Send to self through the account's SMTP relay (STARTTLS, port 587).
    let email = Message::builder()
        .from(account.user.parse().expect("valid from address"))
        .to(account.user.parse().expect("valid to address"))
        .subject(&subject)
        .header(ContentType::TEXT_PLAIN)
        .body(String::from("golem async-imap migration live check"))
        .expect("SHALL build the email");
    let mailer = AsyncSmtpTransport::<Tokio1Executor>::starttls_relay(&account.smtp_host)
        .expect("SHALL build SMTP relay")
        .port(account.smtp_port)
        .credentials(Credentials::new(account.user.clone(), account.pass.clone()))
        .build();
    mailer.send(email).await.expect("SHALL send the email");

    // Receive it through the real async-imap path: rustls handshake, IMAP
    // login, EXAMINE INBOX, FETCH, and RFC822 parse.
    let poller = ImapPoller::new(
        account.imap_host,
        account.imap_port,
        account.user,
        account.pass,
    );
    let msg = poller
        .await_email(&format!("*{subject}*"), 30_000, 2_000)
        .await
        .expect("SHALL receive the sent message");

    assert_eq!(msg.subject, subject, "SHALL receive the message we sent");
    eprintln!(
        "LIVE received subject={:?} from={:?}",
        msg.subject, msg.from
    );
}
