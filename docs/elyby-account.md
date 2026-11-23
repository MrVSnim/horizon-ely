# Ely.by accounts

Settings → Account → **Sign in with Ely.by** (or the button in the top bar) opens the system
browser. The launcher uses Authorization Code with **S256 PKCE**, a random state, and a
`http://127.0.0.1:<port>/oauth/ely` loopback callback (the registered port when it is free, an
OS-assigned one otherwise). It checks the exact callback path and state, and
`iss=https://account.ely.by` when Ely.by sends an `iss`, then sends the code and the private
verifier directly to Ely.by over HTTPS. Codes, verifiers and bearer tokens never follow HTTP
redirects. No hosted backend or client secret is used.

Credentials and two-factor codes stay on Ely.by. User access/refresh tokens are
kept by the existing account secret store. Cancelling, timeout and rejected
callbacks do not add or select an account. Microsoft sign-in is unchanged.

## Release maintainer: register a public Desktop application

1. Open https://account.ely.by/dev and create an application of type **Desktop application**.
2. Copy its public client ID. A Website registration such as the old `elyhorizon2`
   is confidential and still requires a secret; omitting that secret does not turn
   it into a public client. Create a Desktop registration instead.
3. In your GitHub repository, open **Settings → Secrets and variables → Actions → Variables**.
   Add `HORIZON_ELY_CLIENT_ID` with that public Desktop ID.
4. Push to `main`, or run the *Build and Release Elyhorizon* workflow by hand. It compiles the
   public ID into the launcher and publishes `launcher@<version>-ely.<run>` with the launcher
   archive the built-in self-update installs from.

Only this public ID is distributed. **Do not set or embed HORIZON_ELY_CLIENT_SECRET**;
this launcher no longer reads it. No Ely.by secret belongs in Actions Secrets,
source, installers or release artifacts. Rotate previously disclosed Website
credentials in Ely.by and use the public Desktop registration for new releases.

Users of a correctly built release simply click Sign in with Ely.by; no environment variables or
application setup is required on their computer. For source development,
`HORIZON_ELY_CLIENT_ID` can be supplied when running Cargo and will be compiled into that build.
A runtime override takes precedence.

A missing ID reports a release configuration error; `invalid_client` reports that the
maintainer must use a public Desktop registration, rather than asking the player for a secret.
Provider error descriptions are never shown. A refresh failure keeps the selected account; an
invalid or expired grant asks the player to sign in again.

## Provider support and verification

The older [OAuth guide](https://docs.ely.by/en/oauth.html) describes Website clients
and is incomplete for Desktop apps. The live
[discovery document](https://account.ely.by/.well-known/openid-configuration)
advertises S256 and token authentication method `none`.

We inspected the official Apache-2.0 Ely.by implementations as protocol references,
not as copied implementation code:

- [accounts at d4b7f3e](https://github.com/elyby/accounts/tree/d4b7f3eff3572de5f917968321ddc6c007474da0): public Desktop client authentication, loopback redirects, discovery and PKCE tests.
- [accounts-frontend at 7ab69d0c](https://github.com/elyby/accounts-frontend/tree/7ab69d0c7d258538f8560df7ad209fe8becd0620): forwarding authorization parameters, including PKCE, through browser validation/completion.
- [RFC 7636](https://www.rfc-editor.org/rfc/rfc7636): S256 algorithm and independent test vector.

Before calling a release verified, install it on a clean Windows PC, complete the browser
consent, check the selected profile, restart the launcher, press Play, and check the skin in
single-player and on an Ely.by-enabled server. The release workflow also runs the account tests
(`cargo test --release --manifest-path launcher/Cargo.toml --lib 'account::'`) and reports the
result without blocking the release; they do not replace that check.

## How the game uses an Ely.by account

This fork ships no game client of its own: the game is the official, signed Horizon build. As
authlib-injector does for Java, the launcher only points that game at Ely.by when an Ely.by
account plays:

- `HORIZON_API_DISCOVERY` names a services discovery document that the launcher serves on a
  loopback port (`ely::discovery_url`). The game takes its session endpoints from it (join,
  hasJoined, profile lookup) and the hosts it may fetch skins from. The document has no `player`
  service, so the game's account-status and social calls find nothing to ask. The variable is
  described in [microsoft-account.md](microsoft-account.md).
- The session file says `user_type: "msa"`, the only value the official game accepts. What sends
  its requests to Ely.by is the discovery document, not the session file.
- `HORIZON_OFFLINE_SKIN`, `HORIZON_OFFLINE_SKIN_MODEL` and `HORIZON_OFFLINE_CAPE` carry the Ely.by
  skin and cape, downloaded from Ely.by's own hosts only, for single-player, where neither a
  server nor the account supplies one. Other players' skins come from the server as usual.
- A LAN or Play with Friends world verifies its guests through the same document.

The launcher has to be running until the game has read the document, which it does once, at its
first need. If the launcher cannot start the loopback server, the account plays offline: the
session is not handed over, so the token never reaches Mojang's services.

What the official game does not let the launcher change: its own-profile lookup and Realms use
fixed Mojang hosts instead of the discovery document and are sent the Ely.by token there. Both
just fail. Skipping them while `HORIZON_API_DISCOVERY` is set would be a small change in the game
itself. Ely.by issues no Mojang secure-chat certificates. All accounts of one game start follow
the playing account's provider, so splitscreen with mixed providers is not supported.

An Ely.by account is never an owner of Minecraft: Java Edition, so it does not unlock downloads of
Mojang's files (`account::launch` stays strict for them). Those files come from the player's own
installation, as the README says; Ely.by supplies identity and textures only.

## Accounts and servers

The account switcher supports Microsoft and Ely.by together. `accounts.json` uses
`ely:<undashed UUID>` for an Ely.by account's launcher identity; the game receives the real
UUID. The `owner` flag keeps meaning Microsoft ownership, not an Ely.by sign-in. The existing
`account select`, `refresh`, `session` and `sign-out` commands accept the namespaced account ID.
Old Ely.by Yggdrasil sessions remain refreshable; signing in again uses the browser OAuth flow.

Sign out removes local credentials even while offline. Ely.by does not document OAuth token
revocation; revoke the app in your Ely.by account's applications page to end its remote access.

Ely.by accounts join servers that verify players with Ely.by, not Microsoft-only online servers.
A Horizon server reads the same `HORIZON_API_DISCOVERY` (see
[microsoft-account.md](microsoft-account.md)): point it at a document like the launcher's
(`discovery_doc` in `launcher/src/account/ely.rs`), keep `online-mode=true` and set
`enforce-secure-profile=false`, since Ely.by does not issue Mojang secure-chat certificates.
Authentication never falls back between identity providers.

Protocol references (documentation only):

- https://docs.ely.by/en/oauth.html
- https://docs.ely.by/en/minecraft-auth.html
- https://docs.ely.by/en/skins-system.html
- https://docs.ely.by/en/authlib-injector.html
