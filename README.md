# zoho-mail-sync

A Unix-style CLI that mirrors a Zoho Mail account to a local Maildir.

The result is plain `.eml` files on disk that any UNIX mail client — `mutt`, `neomutt`, `mu`, `notmuch`, `aerc`, `mblaze` — can read directly. One-way only: changes on Zoho propagate down; the local copy is read-only in practice (anything you delete locally comes back on the next sync).

## Install

You need Rust and OpenSSL. The repo includes a Nix flake that supplies both:

```sh
nix develop
cargo install --path .
```

## First-time setup

1. Visit <https://api-console.zoho.com/> and create a **Self Client**.
2. Copy `zoho-mail-sync.example.toml` to `zoho-mail-sync.toml` next to where you'll run the tool, and fill in your `client_id` and `client_secret`. Optional fields:
   - `data_dir` — where to store the Maildir. Defaults to the current working directory.
   - `accounts_url` — OAuth host. Defaults to `https://accounts.zoho.com`. Use `https://accounts.zoho.eu`, `.in`, `.com.au`, `.com.cn`, or `.jp` for other data centers.
   - `api_url` — Mail API host. Defaults to `https://mail.zoho.com`. Same regional suffixes apply.
3. In the API Console, click **Generate Code** with these scopes:
   ```
   ZohoMail.accounts.READ,ZohoMail.folders.READ,ZohoMail.messages.READ
   ```
   Pick a 10-minute duration. Copy the code.
4. Exchange it for a long-lived refresh token:
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

## License

MIT.
