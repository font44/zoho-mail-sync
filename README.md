# zoho-mail-sync

A Unix-style CLI that mirrors a Zoho Mail account to a local Maildir.

The result is plain `.eml` files on disk that any UNIX mail client — `mutt`, `neomutt`, `mu`, `notmuch`, `aerc`, `mblaze` — can read directly. One-way only: changes on Zoho propagate down; the local copy is read-only in practice (anything you delete locally comes back on the next sync).

This is not a backup tool on its own — it produces a live mirror that follows deletions on Zoho. Point any backup tool you already use (`restic`, `borg`, `rsnapshot`, Time Machine, etc.) at the data directory to retain history.

## Install

Linux x86_64 tarballs are published on the [GitHub Releases page](https://github.com/font44/zoho-mail-sync/releases). To install from source, use [Nix](https://nixos.org/download/) with flakes enabled and devenv:

```sh
nix profile add github:cachix/devenv/v2.3.1
git clone https://github.com/font44/zoho-mail-sync.git
cd zoho-mail-sync
devenv shell cargo install --locked --path .
```

## First-time setup

1. Visit <https://api-console.zoho.com/> and create a **Self Client**. Copy the client ID and secret.
2. For local use, copy the checked-in dummy environment to the ignored local override and replace both values:
   ```sh
   cp .env .env.local
   ```
   ```dotenv
   ZOHO_CLIENT_ID=1000.XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX
   ZOHO_CLIENT_SECRET=0123456789abcdef...
   ```
   Devenv loads `.env` as non-secret defaults and loads `.env.local` at shell activation so the local values win without being copied into the Nix store. Never put real credentials in the checked-in `.env`. For systemd, set the same variables through `EnvironmentFile`.
3. (Optional) Copy `zoho-mail-sync.example.toml` to `zoho-mail-sync.toml` next to where you'll run the tool to override any of:
   - `data_dir` — where to store the Maildir. Defaults to the current working directory.
   - `accounts_url` — OAuth host. Defaults to `https://accounts.zoho.com`. Use `https://accounts.zoho.eu`, `.in`, `.com.au`, `.com.cn`, or `.jp` for other data centers.
   - `api_url` — Mail API host. Defaults to `https://mail.zoho.com`. Same regional suffixes apply.
   - `[concurrency]` — tunables for parallelism, rate limit, retries, page size; see the example file.

   The TOML file is entirely optional; if absent, defaults apply.
4. In the API Console, click **Generate Code** with these scopes:
   ```
   ZohoMail.accounts.READ,ZohoMail.folders.READ,ZohoMail.messages.READ
   ```
   Pick a 10-minute duration. Copy the code.
5. Exchange it for a long-lived refresh token:
   ```sh
   zoho-mail-sync auth --code <paste-here>
   ```

You won't need the API Console again unless the refresh token is revoked.

## Sync

```sh
zoho-mail-sync sync
```

The first run downloads everything. Later runs are incremental: new messages are fetched, flag changes and folder moves are pure renames, deletions on Zoho are reflected locally. Run it from `cron` or a `systemd` timer.

## Layout

```
<data_dir>/
  .Inbox/{cur,new,tmp}/
  .Sent/{cur,new,tmp}/
  .Trash/{cur,new,tmp}/
  ...                              # one Maildir per Zoho folder
  .zoho-mail-sync/
    tokens.json                    # refresh token, mode 0600
    meta.json                      # account id, folder id ↔ name
```

Each message is a single file named `<zohoMessageId>:2,<flags>` containing the original RFC822 bytes (headers, body, attachments — everything Zoho returns from `originalmessage`). Standard Maildir flags: `S` seen, `F` flagged, `T` trashed, `D` draft.

## Browse

Open the data dir in any Maildir-aware client:

```sh
neomutt -f ~/Mail/.Inbox
mu index --maildir=~/Mail && mu find subject:invoice
notmuch new && notmuch search from:alice
```

## Contributing

The repo ships an `.envrc` configured for devenv. Run `direnv allow` once and the development environment loads automatically. Without direnv, use `devenv shell`.

Run `devenv test` for the same build and unit-test gate used by CI. Future integration tests belong in that gate so Renovate cannot merge without running them.

## Releases

Tagging `vX.Y.Z` triggers a GitHub Actions workflow that builds the release binary inside devenv and attaches a Linux x86_64 tarball to the GitHub Release. Bump `version` in `Cargo.toml` before tagging.

## License

MIT.
