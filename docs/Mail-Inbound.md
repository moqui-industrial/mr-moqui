# Inbound mail for hosted and shared accounts (the SMTP listener)

A **hosted** account (`EmailAccount` kind `EakHosted`, an address at a domain the business hosts) and a **shared** account
(`EakShared`, such as orders@ or support@, with members) get their mail from the SMTP listener: the last hop behind an
**edge gateway** that does the internet side. The gateway (Postfix with Rspamd, or another MTA) takes the mail from the
world with TLS, checks SPF, DKIM and DMARC, filters spam and viruses, queues and retries, and hands each message to
mr-moqui over plain SMTP. The listener stores it in the account's Inbox as the IMAP sync stores fetched mail and runs what
new mail runs. External accounts (Gmail, Microsoft 365, any IMAP or POP server) keep using the sync. As of Oct 19, 2026.

The code is `crates/moqui-host/src/smtp.rs` (the server) and `crates/moqui-core/src/service/mail/delivery.rs` (routing and
delivery). It is the moqui-host feature `inbound-smtp`, in the Debug, Maximal (`--release`) and Enterprise packages
([Packaging](Packaging.md)); the Default package leaves it out, and adds it with:

```
./moq build -- --features inbound-smtp
```

It adds no crate: the server is a small RFC 5321 server on Tokio, for the one client it serves.

## What the deployment configures

The listener starts with the server when `smtp_listen_port` is set. The settings are the attributes of the
`smtp-server` element in `MoquiDefaultConf.xml`; the deployment ones are also properties (`smtp_listen_port` and so on,
in the environment or `runtime/.env`), and the others are changed in a runtime or component conf, such as
`<smtp-server max-message-size="52428800"/>`.

| Property or attribute | Default | |
| --- | --- | --- |
| `smtp_listen_port` | empty (off) | The port the gateway delivers to, such as `2525`. Use a port above 1024: the gateway, not mr-moqui, has port 25. |
| `smtp_listen_host` | `127.0.0.1` | The address to listen on: loopback for a gateway on the same host, the WireGuard or private address for a remote one. |
| `smtp_allowed_addresses` | empty | The gateway's addresses, IPs and CIDRs separated by commas (`127.0.0.1`, `10.8.0.1`, `fd00::/8`). Every other address gets `554` at connect. **Empty does not start the listener**, so it never takes mail from everyone. |
| `smtp-server` `proxy-protocol` | `false` | Each connection starts with a PROXY v1 or v2 header (a TCP proxy such as HAProxy in front of mr-moqui). The address the header names must be in `smtp_allowed_addresses` too, so list both the proxy's and the gateway's addresses. |
| `smtp_hostname` | `webapp_http_host` | The name in the greeting and in the `Received` field. |
| `smtp_authserv_id` | empty | The authserv-id the gateway writes in its `Authentication-Results` (Rspamd: the `authserv_id` of its milter headers, usually the gateway's host name). Only those fields are kept on the message; any other `Authentication-Results` (such as one a sender forged) is dropped. Empty keeps them all as they came, and a warning says so at startup. |
| `smtp-server` `spam-headers` | `X-Spam-Flag,X-Spam-Status,X-Spam` | The gateway's spam verdict fields. A message where one of them starts with `yes` or `true` (`X-Spam-Flag: YES`, `X-Spam-Status: Yes, score=7.2`) goes to the account's Junk folder. |
| `smtp-server` `plus-separator` | `+` | The plus-addressing characters: `sales+news@example.com` goes to `sales@example.com`. Several characters are each a separator (`+-`); `none` turns it off. |
| `smtp-server` `max-message-size` | `26214400` (25 MiB) | Bytes, advertised as `SIZE`. A larger message is refused with `552` (at `MAIL FROM` when its `SIZE` says so, else after its data). A message is held in memory while it is stored. |
| `smtp-server` `max-recipients` | `100` | Recipients of one message; the next gets `452` and the gateway sends to it later. |
| `smtp-server` `max-connections` | `20` | Connections at once; the next gets `421`. |
| `smtp-server` `command-timeout` | `300` | Seconds for each command line and each chunk of data. |
| `smtp-server` `session-timeout` | `1800` | Seconds for a whole connection. |

The accounts are made as usual (`create#EmailAccount` with `accountKindEnumId` `EakHosted` or `EakShared`, the BOSS
MailAccounts wizard). An account gets the listener's mail when it is hosted or shared, not removed, and has **no incoming
server** (no `storeHost` on its `EmailServer`): a shared account the sync feeds from an IMAP mailbox is left to the sync,
or it would get each message twice. Its outgoing server, if it has one, is how its replies and automatic replies go out,
usually back through the gateway.

## DNS and the gateway

- **MX records point at the gateway**, never at mr-moqui: `example.com. MX 10 mx.example.com.` with `mx.example.com` the
  gateway's public address. SPF, DKIM (the gateway signs outbound mail) and DMARC records are the gateway's.
- The gateway relays the hosted domains to mr-moqui. Postfix, with mr-moqui on the same host:

  ```
  relay_domains = example.com
  transport_maps = hash:/etc/postfix/transport      # example.com  smtp:[127.0.0.1]:2525
  smtpd_recipient_restrictions = permit_mynetworks, reject_unauth_destination, reject_unverified_recipient
  ```

  `reject_unverified_recipient` asks mr-moqui (`RCPT TO`) before it accepts mail from the internet, so mail to an address
  no account has is refused at the edge and nothing bounces later (no backscatter). The listener answers `550 5.1.1` for
  such an address.
- **The gateway must remove** `Authentication-Results` fields with its own authserv-id that came from outside (RFC 8601
  section 5; the `remove` setting of Rspamd's `authentication-results` milter header), and the spam verdict fields that came
  from upstream. The listener trusts what the gateway writes and verifies nothing again.
- A remote gateway (a small VPS for a site with port 25 blocked or no fixed address) forwards over WireGuard or another
  private link: set `smtp_listen_host` to mr-moqui's address on that link and `smtp_allowed_addresses` to the gateway's.
  The listener speaks no TLS, so the link between them must be private (loopback, WireGuard, a private network).

## What happens to a message

At `RCPT TO` the address is looked up without regard to case: an account's address, then one of its `EmailIdentity`
addresses, then the same without the plus tag. An address two accounts claim at the same step goes to the account made
first (the lowest id), with a warning naming both, so an identity a user adds later cannot read another account's mail.

After `DATA` the message is stored once in each account it is for, however many of the account's addresses were
recipients:

1. Trace fields go on top: `Return-Path` (the envelope sender; any the message had is removed), a `Delivered-To` for each
   recipient address of the account, and `Received`. Only the gateway's `Authentication-Results` stay.
2. The `EmailMessage` is stored in the Inbox (Junk for the gateway's spam), unread, with its files as DbResource files
   under `dbresource://email/<account>/<message>/` and its place in a thread, in a transaction of its own. An app's
   `EmailMessage` ECA files it on the records by the account's filing policy (Mantle's `file#ReceivedEmail`: every party
   match for hosted and shared accounts; mail in Junk is not filed).
3. For Inbox mail: the email ECAs (`*.emecas.xml`), the account's rules (`run#EmailRules`), the snooze wake
   (`wake#SnoozedThreads`) and the automatic reply (`send#OutOfOfficeReplies`). Then the counts of the account's users.
   Each of these logs its own failure, and the message stays stored.

The listener answers `250` when every account has it. When a store fails (the database is down, the account has no Inbox
folder) or no request worker is free, it answers `451` and the gateway tries again later; an account that stored the
message the first time sees the same Message-ID and does not store it again. A message with no header fields is refused
with `554`.

Each session is logged when it ends (the client and its EHLO name, the messages, recipients accepted and refused, how it
ended), and each message with its sender, recipients, size, the accounts and message ids it was stored as, the spam
verdict and the authentication results in short (`spf=pass dkim=pass dmarc=pass`). Message bodies are never logged.

## What it does not do

No `AUTH`, no `STARTTLS`, no relaying: it takes mail only for its own accounts and only from the gateway. No DSN, `BDAT`
(CHUNKING) or `ETRN`. No SPF, DKIM or DMARC checks, no spam filter of its own, no queue: those are the gateway's. A
`postmaster` mailbox is an account like any other (the gateway answers for its own). `VRFY` answers `252`.
