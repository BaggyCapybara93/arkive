//! TLS-protected prototype transport for Arkive servers.

pub(crate) mod error;
mod files;
mod protocol;
#[cfg(any(target_os = "linux", target_os = "macos"))]
mod server_files;

use std::fs::{self, File};
use std::io::{self, BufReader, IsTerminal, Read, Write};
use std::net::{SocketAddr, TcpListener, TcpStream};
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rustls::pki_types::ServerName;
use rustls::{
    ClientConfig, ClientConnection, RootCertStore, ServerConfig, ServerConnection, StreamOwned,
};
use subtle::ConstantTimeEq;

use crate::error::AppError;
use crate::vault;
use crate::vault::snapshots::{self, RemoteImport};
use error::invalid;
use protocol::{Action, Message};

const IO_TIMEOUT: Duration = Duration::from_secs(60);

trait ReadWrite: Read + Write {}
impl<T: Read + Write> ReadWrite for T {}

fn private_file(path: &Path) -> Result<(), AppError> {
    let metadata = fs::symlink_metadata(path)?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(invalid(format!(
            "Remote secret must be a regular file: {path:?}"
        )));
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if metadata.permissions().mode() & 0o077 != 0 {
            return Err(invalid(format!(
                "Remote secret must be readable only by its owner (chmod 600): {path:?}"
            )));
        }
    }
    Ok(())
}

fn read_token(path: &Path) -> Result<String, AppError> {
    private_file(path)?;
    let bytes = fs::read(path)?;
    if bytes.len() > 128 {
        return Err(invalid("Remote token file is too large"));
    }
    let text = String::from_utf8(bytes)
        .map_err(|_| invalid("Remote token file must contain UTF-8 text"))?;
    let token = text.trim();
    if token.len() != 64 || !token.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        return Err(invalid("Remote token must be 64 hexadecimal characters"));
    }
    Ok(token.to_ascii_lowercase())
}

fn certificate_chain(
    path: &Path,
) -> Result<Vec<rustls::pki_types::CertificateDer<'static>>, AppError> {
    let mut reader = BufReader::new(File::open(path)?);
    let certs = rustls_pemfile::certs(&mut reader).collect::<Result<Vec<_>, _>>()?;
    if certs.is_empty() {
        return Err(invalid("Remote certificate file contains no certificates"));
    }
    Ok(certs)
}

fn server_config(cert: &Path, key: &Path) -> Result<Arc<ServerConfig>, AppError> {
    let certs = certificate_chain(cert)?;
    private_file(key)?;
    let mut reader = BufReader::new(File::open(key)?);
    let key = rustls_pemfile::private_key(&mut reader)?
        .ok_or_else(|| invalid("Remote key file contains no private key"))?;
    let config = ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(certs, key)
        .map_err(|error| invalid(format!("Invalid remote TLS certificate: {error}")))?;
    Ok(Arc::new(config))
}

fn client_config(cert: &Path) -> Result<Arc<ClientConfig>, AppError> {
    let mut roots = RootCertStore::empty();
    for certificate in certificate_chain(cert)? {
        roots
            .add(certificate)
            .map_err(|error| invalid(format!("Invalid trusted remote certificate: {error}")))?;
    }
    Ok(Arc::new(
        ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth(),
    ))
}

fn connect(
    address: SocketAddr,
    cert: &Path,
) -> Result<StreamOwned<ClientConnection, TcpStream>, AppError> {
    let tcp = TcpStream::connect_timeout(&address, IO_TIMEOUT)?;
    tcp.set_read_timeout(Some(IO_TIMEOUT))?;
    tcp.set_write_timeout(Some(IO_TIMEOUT))?;
    let name = ServerName::IpAddress(address.ip().into());
    let connection = ClientConnection::new(client_config(cert)?, name)
        .map_err(|error| invalid(format!("Could not start remote TLS connection: {error}")))?;
    Ok(StreamOwned::new(connection, tcp))
}

fn hello(stream: &mut impl ReadWrite, token: String, action: Action) -> Result<String, AppError> {
    protocol::write(
        stream,
        &Message::Hello {
            version: protocol::VERSION,
            token,
            action,
        },
    )?;
    match protocol::read(stream)? {
        Message::Ready { vault_id } => Ok(vault_id),
        Message::Error { message } => {
            Err(invalid(format!("Remote rejected connection: {message}")))
        }
        _ => Err(invalid("Unexpected remote handshake response")),
    }
}

fn confirmed(answer: &str) -> bool {
    matches!(answer.trim().to_ascii_lowercase().as_str(), "y" | "yes")
}

fn confirm_upload(address: SocketAddr) -> Result<(), AppError> {
    if !io::stdin().is_terminal() || !io::stderr().is_terminal() {
        return Err(invalid(
            "Remote upload requires an interactive terminal for input and prompts",
        ));
    }
    eprintln!("Remote is a prototype feature. You are sending data to {address}.");
    eprint!("Do you wish to continue? [y/N] ");
    io::stderr().flush()?;
    let mut answer = String::new();
    io::stdin().read_line(&mut answer)?;
    if confirmed(&answer) {
        Ok(())
    } else {
        Err(invalid("Remote upload cancelled before connecting"))
    }
}

pub fn check(address: SocketAddr, cert: &Path, token_file: &Path) -> Result<(), AppError> {
    let token = read_token(token_file)?;
    let mut stream = connect(address, cert)?;
    let vault_id = hello(&mut stream, token, Action::Check)?;
    println!("Remote at {address} is ready (vault {vault_id})");
    Ok(())
}

pub fn list(
    address: SocketAddr,
    cert: &Path,
    token_file: &Path,
    path: &str,
) -> Result<(), AppError> {
    files::validate_relative(path, true)?;
    let token = read_token(token_file)?;
    let mut stream = connect(address, cert)?;
    hello(&mut stream, token, Action::List)?;
    protocol::write(&mut stream, &Message::List { path: path.into() })?;
    match protocol::read(&mut stream)? {
        Message::Listing { entries } => {
            for entry in entries {
                println!("{}  {}  {}", entry.kind, entry.size, entry.name);
            }
            Ok(())
        }
        Message::Error { message } => Err(invalid(format!("Remote list failed: {message}"))),
        _ => Err(invalid("Unexpected remote list response")),
    }
}

pub fn copy(
    source: &Path,
    destination: &str,
    address: SocketAddr,
    cert: &Path,
    token_file: &Path,
    dry_run: bool,
) -> Result<(), AppError> {
    files::validate_relative(destination, false)?;
    if dry_run {
        let export = files::LocalExport::scan(source)?;
        let (count, bytes) = export.manifest.file_count_and_bytes()?;
        println!(
            "[DRY-RUN] Would copy {count} file(s), {bytes} byte(s) from {source:?} to {address}:{destination}"
        );
        return Ok(());
    }
    confirm_upload(address)?;
    let export = files::LocalExport::scan(source)?;
    let token = read_token(token_file)?;
    let mut stream = connect(address, cert)?;
    hello(&mut stream, token, Action::Copy)?;
    protocol::write(
        &mut stream,
        &Message::Copy {
            destination: destination.into(),
            manifest_size: export.bytes.len(),
        },
    )?;
    stream.write_all(&export.bytes)?;
    stream.flush()?;
    match protocol::read(&mut stream)? {
        Message::TransferReady => {}
        Message::Error { message } => {
            return Err(invalid(format!("Remote copy refused: {message}")));
        }
        _ => return Err(invalid("Unexpected remote copy response")),
    }
    let sent = export.send(&mut stream)?;
    match protocol::read(&mut stream)? {
        Message::FileComplete { bytes } if bytes == sent => {
            println!("Copied {sent} byte(s) to {address}:{destination}");
            Ok(())
        }
        Message::Error { message } => Err(invalid(format!("Remote copy failed: {message}"))),
        _ => Err(invalid("Unexpected remote copy completion")),
    }
}

pub fn download(
    source: &str,
    destination: &Path,
    address: SocketAddr,
    cert: &Path,
    token_file: &Path,
    dry_run: bool,
) -> Result<(), AppError> {
    files::validate_relative(source, false)?;
    files::validate_destination(destination)?;
    if dry_run {
        println!("[DRY-RUN] Would download {address}:{source} to {destination:?}");
        return Ok(());
    }
    let token = read_token(token_file)?;
    let mut stream = connect(address, cert)?;
    hello(&mut stream, token, Action::Download)?;
    protocol::write(
        &mut stream,
        &Message::Download {
            source: source.into(),
        },
    )?;
    let size = match protocol::read(&mut stream)? {
        Message::FileManifest { manifest_size }
            if manifest_size > 0 && manifest_size <= protocol::MAX_MANIFEST_BYTES =>
        {
            manifest_size
        }
        Message::Error { message } => {
            return Err(invalid(format!("Remote download refused: {message}")));
        }
        _ => return Err(invalid("Unexpected remote download response")),
    };
    let mut bytes = vec![0u8; size];
    stream.read_exact(&mut bytes)?;
    let manifest = files::Manifest::decode(&bytes)?;
    files::validate_destination(destination)?;
    protocol::write(&mut stream, &Message::TransferReady)?;
    let received = files::receive_to(&mut stream, destination, &manifest)?;
    match protocol::read(&mut stream)? {
        Message::FileComplete { bytes } if bytes == received.bytes() => {
            received.publish()?;
            println!("Downloaded {bytes} byte(s) to {destination:?}");
            Ok(())
        }
        Message::Error { message } => Err(invalid(format!("Remote download failed: {message}"))),
        _ => Err(invalid("Unexpected remote download completion")),
    }
}

pub fn push(
    vault_path: &Path,
    snapshot_id: &str,
    address: SocketAddr,
    cert: &Path,
    token_file: &Path,
    dry_run: bool,
) -> Result<(), AppError> {
    if dry_run {
        let export = snapshots::remote_export(vault_path, snapshot_id)?;
        println!(
            "[DRY-RUN] Would push snapshot {snapshot_id} ({} object(s)) to {address}",
            export.objects.len()
        );
        return export.release();
    }
    confirm_upload(address)?;
    let export = snapshots::remote_export(vault_path, snapshot_id)?;
    let result = (|| {
        let token = read_token(token_file)?;
        let mut stream = connect(address, cert)?;
        push_stream(&mut stream, &export, snapshot_id, token)
    })();
    let release = export.release();
    let (remote_vault, objects, sent) = match (result, release) {
        (Ok(value), Ok(())) => value,
        (Err(error), Ok(())) => return Err(error),
        (Ok(_), Err(error)) => return Err(error),
        (Err(error), Err(release)) => {
            return Err(invalid(format!(
                "{error}; releasing local vault lock also failed: {release}"
            )));
        }
    };
    println!(
        "Pushed snapshot {snapshot_id} to vault {remote_vault}: {objects} object(s), {sent} byte(s) uploaded"
    );
    Ok(())
}

fn push_stream(
    stream: &mut impl ReadWrite,
    export: &snapshots::RemoteExport,
    snapshot_id: &str,
    token: String,
) -> Result<(String, usize, u64), AppError> {
    let remote_vault = hello(stream, token, Action::Push)?;
    protocol::write(
        stream,
        &Message::Push {
            snapshot_id: snapshot_id.into(),
            manifest_size: export.manifest.len(),
        },
    )?;
    stream.write_all(&export.manifest)?;
    stream.flush()?;
    let missing = match protocol::read(stream)? {
        Message::Missing { objects } => objects,
        Message::Error { message } => {
            return Err(invalid(format!("Remote rejected snapshot: {message}")));
        }
        _ => return Err(invalid("Unexpected remote inventory response")),
    };
    let mut sent = 0u64;
    for (hash, size) in &missing {
        let object = export.open_object(hash, *size)?;
        let copied = io::copy(&mut object.take(*size), stream)?;
        if copied != *size {
            return Err(invalid(format!("Local object {hash} ended early")));
        }
        sent = sent
            .checked_add(copied)
            .ok_or_else(|| invalid("Upload size overflow"))?;
    }
    stream.flush()?;
    match protocol::read(stream)? {
        Message::SnapshotComplete {
            snapshot_id: received,
        } if received == snapshot_id => Ok((remote_vault, missing.len(), sent)),
        Message::Error { message } => Err(invalid(format!("Remote push failed: {message}"))),
        _ => Err(invalid("Unexpected remote completion response")),
    }
}

fn serve_snapshot(stream: &mut impl ReadWrite, vault_path: &Path) -> Result<(), AppError> {
    let (snapshot_id, manifest_size) = match protocol::read(stream)? {
        Message::Push {
            snapshot_id,
            manifest_size,
        } => (snapshot_id, manifest_size),
        _ => return Err(invalid("Remote client did not send a snapshot")),
    };
    if manifest_size == 0 || manifest_size > protocol::MAX_MANIFEST_BYTES {
        protocol::write(
            stream,
            &Message::Error {
                message: "Invalid snapshot manifest size".into(),
            },
        )?;
        return Err(invalid("Invalid remote snapshot manifest size"));
    }
    let mut manifest = vec![0u8; manifest_size];
    stream.read_exact(&mut manifest)?;
    let import = match RemoteImport::begin(vault_path, &snapshot_id, manifest) {
        Ok(import) => import,
        Err(error) => {
            protocol::write(
                stream,
                &Message::Error {
                    message: error.to_string(),
                },
            )?;
            return Err(error);
        }
    };
    protocol::write(
        stream,
        &Message::Missing {
            objects: import.missing().to_vec(),
        },
    )?;
    for (hash, size) in import.missing() {
        import.receive_object(hash, *size, stream)?;
    }
    let id = import.commit()?;
    protocol::write(stream, &Message::SnapshotComplete { snapshot_id: id })
}

fn serve_stream(
    stream: &mut impl ReadWrite,
    vault_path: &Path,
    vault_id: &str,
    token: &str,
    files_root: &Path,
) -> Result<(), AppError> {
    let action = match protocol::read_limited(stream, protocol::MAX_HELLO_BYTES)? {
        Message::Hello {
            version,
            token: supplied,
            action,
        } => {
            if version != protocol::VERSION {
                protocol::write(
                    stream,
                    &Message::Error {
                        message: "Unsupported protocol version".into(),
                    },
                )?;
                return Err(invalid("Unsupported remote protocol version"));
            }
            if supplied.as_bytes().ct_eq(token.as_bytes()).unwrap_u8() != 1 {
                protocol::write(
                    stream,
                    &Message::Error {
                        message: "Authentication failed".into(),
                    },
                )?;
                return Err(invalid("Remote authentication failed"));
            }
            action
        }
        _ => return Err(invalid("Remote client did not send a handshake")),
    };
    if vault::remote_identity(vault_path)? != vault_id {
        protocol::write(
            stream,
            &Message::Error {
                message: "Server vault changed after startup".into(),
            },
        )?;
        return Err(invalid("Server vault changed after startup"));
    }
    protocol::write(
        stream,
        &Message::Ready {
            vault_id: vault_id.into(),
        },
    )?;
    match action {
        Action::Check => Ok(()),
        Action::Push => serve_snapshot(stream, vault_path),
        Action::List | Action::Copy | Action::Download => serve_files(stream, files_root, action),
    }
}

#[cfg(any(target_os = "linux", target_os = "macos"))]
fn serve_files(
    stream: &mut impl ReadWrite,
    files_root: &Path,
    action: Action,
) -> Result<(), AppError> {
    let result = (|| {
        let root = server_files::FilesRoot::open(files_root)?;
        match action {
            Action::List => {
                let path = match protocol::read(stream)? {
                    Message::List { path } => path,
                    _ => return Err(invalid("Expected remote list request")),
                };
                let entries = root.list(&path)?;
                protocol::write(stream, &Message::Listing { entries })
            }
            Action::Copy => {
                let (destination, size) = match protocol::read(stream)? {
                    Message::Copy {
                        destination,
                        manifest_size,
                    } => (destination, manifest_size),
                    _ => return Err(invalid("Expected remote copy request")),
                };
                if size == 0 || size > protocol::MAX_MANIFEST_BYTES {
                    return Err(invalid("Invalid file manifest size"));
                }
                let mut bytes = vec![0u8; size];
                stream.read_exact(&mut bytes)?;
                let manifest = files::Manifest::decode(&bytes)?;
                let stage = root.begin_upload(&destination, manifest)?;
                protocol::write(stream, &Message::TransferReady)?;
                let written = stage.receive_and_publish(stream)?;
                protocol::write(stream, &Message::FileComplete { bytes: written })
            }
            Action::Download => {
                let source = match protocol::read(stream)? {
                    Message::Download { source } => source,
                    _ => return Err(invalid("Expected remote download request")),
                };
                let export = root.export(&source)?;
                protocol::write(
                    stream,
                    &Message::FileManifest {
                        manifest_size: export.manifest_bytes().len(),
                    },
                )?;
                stream.write_all(export.manifest_bytes())?;
                stream.flush()?;
                match protocol::read(stream)? {
                    Message::TransferReady => {}
                    _ => return Err(invalid("Expected download readiness")),
                }
                let sent = export.send(stream)?;
                stream.flush()?;
                protocol::write(stream, &Message::FileComplete { bytes: sent })
            }
            Action::Check | Action::Push => unreachable!(),
        }
    })();
    if let Err(error) = &result {
        let _ = protocol::write(
            stream,
            &Message::Error {
                message: error.to_string(),
            },
        );
    }
    result
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn serve_files(
    _stream: &mut impl ReadWrite,
    _files_root: &Path,
    _action: Action,
) -> Result<(), AppError> {
    Err(invalid(
        "Remote file storage is not supported by this server platform yet",
    ))
}

pub fn serve(
    vault_path: &Path,
    listen: SocketAddr,
    cert: &Path,
    key: &Path,
    token_file: &Path,
    files_root: Option<&Path>,
) -> Result<(), AppError> {
    let vault_id = vault::remote_identity(vault_path)?;
    let files_root = files::prepare_root(vault_path, files_root)?;
    let token = read_token(token_file)?;
    let config = server_config(cert, key)?;
    let listener = TcpListener::bind(listen)?;
    println!(
        "Remote prototype serving vault {vault_id} at {} (files: {})",
        listener.local_addr()?,
        files_root.display()
    );
    for incoming in listener.incoming() {
        let tcp = incoming?;
        tcp.set_read_timeout(Some(IO_TIMEOUT))?;
        tcp.set_write_timeout(Some(IO_TIMEOUT))?;
        let connection = ServerConnection::new(config.clone())
            .map_err(|error| invalid(format!("Could not start remote TLS session: {error}")))?;
        let mut stream = StreamOwned::new(connection, tcp);
        if let Err(error) = serve_stream(&mut stream, vault_path, &vault_id, &token, &files_root) {
            eprintln!("Remote connection failed: {error}");
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests;
