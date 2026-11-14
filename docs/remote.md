# Remote prototype

The built-in Arkive server can receive a vault snapshot or hold ordinary files
for another Arkive client. This is a separate feature from batch jobs. It does
not automatically sync hosts or resolve conflicts.

## Set up a server

Initialize a vault on the server, then create a private token and a TLS
certificate whose subject alternative name contains the server IP address.
For a local test with `127.0.0.1`:

```bash
mkdir -p ./server-vault
arkive vault init ./server-vault
umask 077
openssl rand -hex 32 > ./remote-token.txt
openssl req -x509 -newkey rsa:3072 -nodes -days 30 \
  -keyout ./remote-key.pem -out ./remote-cert.pem \
  -subj '/CN=Arkive prototype' \
  -addext 'subjectAltName=IP:127.0.0.1' \
  -addext 'basicConstraints=critical,CA:FALSE' \
  -addext 'extendedKeyUsage=serverAuth'
arkive remote serve ./server-vault --listen 127.0.0.1:7474 \
  --cert ./remote-cert.pem --key ./remote-key.pem \
  --token-file ./remote-token.txt
```

Copy the certificate and token to the client over a trusted channel. The
client trusts the supplied certificate and validates the server IP against it.
Protect the token and private key with owner-only file permissions, such as
`chmod 600`. Use a firewall or trusted network to restrict access to the
listening address.

By default the server creates `<VAULT>/remote-files` with restricted
permissions. To use an existing directory instead, set `--files-root PATH` on
`remote serve`. The server requires this directory to be owned by its running
user and not writable by other users. It cannot be the vault, contain the
vault, or point into `.arkive-vault`. Clients cannot choose or change the
server's files root.

## Use the server

Each client command needs `--address`, `--cert`, and `--token-file`:

```bash
arkive remote check --address 127.0.0.1:7474 \
  --cert ./remote-cert.pem --token-file ./remote-token.txt

arkive remote list --address 127.0.0.1:7474 \
  --cert ./remote-cert.pem --token-file ./remote-token.txt

arkive remote copy ./my-saves backups/my-saves \
  --address 127.0.0.1:7474 \
  --cert ./remote-cert.pem --token-file ./remote-token.txt

arkive remote download backups/my-saves ./downloaded-saves \
  --address 127.0.0.1:7474 \
  --cert ./remote-cert.pem --token-file ./remote-token.txt

arkive remote push ./local-vault SNAPSHOT_ID \
  --address 127.0.0.1:7474 \
  --cert ./remote-cert.pem --token-file ./remote-token.txt
```

`remote copy` and `remote push` require an interactive confirmation before
connecting: “Remote is a prototype feature. You are sending data to
`IP:PORT`. Do you wish to continue? [y/N]” Only `y` or `yes` proceeds.
`arkive --dry-run remote copy ...` and `arkive --dry-run remote push ...`
preview local transfer size without connecting or asking for confirmation.

Remote file paths such as `backups/my-saves` are relative to the server files
root. `remote list` can list that root or a subdirectory. `remote copy` accepts
a local file or directory and creates a new server destination. `remote
download` creates a new local destination. Existing destinations are never
replaced. Transfers verify SHA-256 hashes and stage data before publication.
The server refuses traversal paths, symlinks, and special files inside its
files root. Uploaded file paths cannot escape this root.

Snapshot pushes use the vault's immutable snapshot IDs and content-addressed
objects. The receiver verifies each object and the manifest before publishing
the snapshot. The remote file area is ordinary storage, outside snapshot
history and outside the snapshot quota.

## Current limits

- The server handles one connection at a time and has no resume support.
- File area storage has no quota; monitor disk capacity separately.
- File transfers do not preserve permissions, ownership, timestamps, ACLs, or extended attributes.
- The server file area currently runs on Linux and macOS, using directory-handle operations.
- Snapshot push is one-way. Remote download applies only to ordinary files,
  not snapshots; use `vault restore` on the receiving host for a pushed snapshot.
