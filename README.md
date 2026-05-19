# zoho-mail-sync

A Unix-style CLI that mirrors a Zoho Mail account to a local Maildir.

The result is plain `.eml` files on disk that any UNIX mail client — `mutt`, `neomutt`, `mu`, `notmuch`, `aerc`, `mblaze` — can read directly. One-way only: changes on Zoho propagate down; the local copy is read-only in practice (anything you delete locally comes back on the next sync).

## Install

Built and distributed as a Nix flake. Requires a working [Nix](https://nixos.org/download/) install with flakes enabled.

```sh
nix profile add github:font44/zoho-mail-sync
```

## First-time setup

1. Visit <https://api-console.zoho.com/> and create a **Self Client**. Copy the client ID and secret.
2. Export them in your shell, `.envrc`, or systemd `EnvironmentFile`:
   ```sh
   export ZOHO_CLIENT_ID=1000.XXXXXXXXXXXXXXXXXXXXXXXXXXXXXXXX
   export ZOHO_CLIENT_SECRET=0123456789abcdef...
   ```
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

The repo ships an `.envrc` with `use flake`; run `direnv allow` once and the dev shell loads automatically. Without direnv, use `nix develop`.

## Releases

Tagging `vX.Y.Z` triggers a GitHub Actions workflow that builds the binary via `nix build` and attaches a Linux x86_64 tarball to the GitHub Release. Bump `version` in `Cargo.toml` and `flake.nix` together before tagging.

## License

MIT.
