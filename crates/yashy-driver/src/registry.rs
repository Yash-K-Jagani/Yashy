//! A package registry: a directory on this machine, or a URL.
//!
//! The registry is a directory with `index.json` and one uncompressed
//! `.tar` file per tree hash under `packages/`. Publishing writes the
//! tarball through a staging file and renames it into place, and writes the
//! index the same way. A duplicate name-and-version with a different hash is
//! an error; the same name-and-version and hash is an idempotent republish.
//!
//! A *read* may also come from a URL, because the registry a release
//! publishes is a static index on a host and not a directory on the reader's
//! machine. Publishing stays local: `file://` is where a package is written,
//! and a pull request to the registry's repository is how a package gets
//! there.

use std::path::{Path, PathBuf};
use std::process::Command;

use yashy_common::Json;
use yashy_manifest::{hash_tree, Manifest};

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryEntry {
    pub name: String,
    pub version: String,
    pub hash: String,
    pub description: String,
}

/// Where a registry lives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RegistrySource {
    /// A directory on this machine.
    Local(PathBuf),
    /// A URL: `http://` for a host on this machine, `https://` for one that is
    /// not.
    Remote(String),
}

impl RegistrySource {
    /// How the source is named in a message.
    pub fn describe(&self) -> String {
        match self {
            RegistrySource::Local(path) => path.display().to_string(),
            RegistrySource::Remote(url) => url.clone(),
        }
    }

    /// Whether the source is on this machine.
    pub fn is_local(&self) -> bool {
        matches!(self, RegistrySource::Local(_))
    }
}

/// The registry a spec names.
///
/// A spec is a directory, a `file://` URL, or an `http(s)://` URL. The URL
/// forms are for *reading*: publishing needs a directory it can write to.
pub fn registry_source(spec: &str) -> Result<RegistrySource, RegistryError> {
    if let Some(rest) = spec.strip_prefix("file://") {
        return Ok(RegistrySource::Local(PathBuf::from(rest)));
    }
    if spec.starts_with("http://") || spec.starts_with("https://") {
        return Ok(RegistrySource::Remote(spec.to_string()));
    }
    if Path::new(spec).is_absolute() {
        return Ok(RegistrySource::Local(PathBuf::from(spec)));
    }
    Err(registry_error(format!(
        "registry must be an absolute path, a file URL, or an http URL, not `{spec}`"
    )))
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RegistryError {
    pub message: String,
}

impl std::fmt::Display for RegistryError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str(&self.message)
    }
}

fn registry_error(message: impl Into<String>) -> RegistryError {
    RegistryError {
        message: message.into(),
    }
}

pub fn registry_path(spec: &str) -> Result<PathBuf, RegistryError> {
    if let Some(rest) = spec.strip_prefix("file://") {
        let path = if rest.starts_with('/') {
            PathBuf::from(rest)
        } else {
            PathBuf::from(rest)
        };
        if path.is_absolute() {
            return Ok(path);
        }
    }
    if Path::new(spec).is_absolute() {
        return Ok(PathBuf::from(spec));
    }
    Err(registry_error(format!(
        "registry must be an absolute path or a file URL, not `{spec}`"
    )))
}

fn read_index(root: &Path) -> Result<Vec<RegistryEntry>, RegistryError> {
    let path = root.join("index.json");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(Vec::new());
    };
    read_index_text(&text)
}

fn index_json(entries: &[RegistryEntry]) -> String {
    let mut array = Vec::new();
    for entry in entries {
        let mut item = Json::object();
        item.set("name", Json::string(&entry.name));
        item.set("version", Json::string(&entry.version));
        item.set("hash", Json::string(&entry.hash));
        item.set("description", Json::string(&entry.description));
        array.push(item);
    }
    let mut object = Json::object();
    object.set("packages", Json::array(array));
    object.render()
}

pub fn search(root: &Path, query: Option<&str>) -> Result<Vec<RegistryEntry>, RegistryError> {
    let mut entries = read_index(root)?;
    entries.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.version.cmp(&right.version))
    });
    if let Some(query) = query {
        entries.retain(|entry| {
            entry.name.contains(query) || entry.version.contains(query)
        });
    }
    Ok(entries)
}

/// The index of a registry, wherever it lives.
pub fn fetch_index(source: &RegistrySource) -> Result<Vec<RegistryEntry>, RegistryError> {
    match source {
        RegistrySource::Local(root) => read_index(root),
        RegistrySource::Remote(url) => {
            let index_url = format!("{}/index.json", url.trim_end_matches('/'));
            let bytes = fetch_url(&index_url)?;
            let text = String::from_utf8_lossy(&bytes).into_owned();
            read_index_text(&text)
        }
    }
}

/// The tarball of one entry, wherever the registry lives.
pub fn fetch_tarball(
    source: &RegistrySource,
    entry: &RegistryEntry,
) -> Result<Vec<u8>, RegistryError> {
    let hash = entry.hash.trim_start_matches("sha256:");
    match source {
        RegistrySource::Local(root) => {
            let path = root.join("packages").join(format!("{hash}.tar"));
            std::fs::read(&path).map_err(|error| {
                registry_error(format!("cannot read {}: {error}", path.display()))
            })
        }
        RegistrySource::Remote(url) => {
            let tarball_url = format!(
                "{}/packages/{hash}.tar",
                url.trim_end_matches('/')
            );
            fetch_url(&tarball_url)
        }
    }
}

/// Fetch a URL.
///
/// The transport is `curl`, and the reason is TLS: the compiler ships no TLS
/// stack, and a registry fetched over plaintext is a registry whose packages
/// can be replaced in flight. `--proto =https` is what makes the refusal
/// real -- it tells curl not to follow a redirect to a plaintext URL, and not
/// to speak plaintext at all when the URL says `https`.
///
/// A machine without `curl` gets a message that names it, rather than a
/// silent fallback to something that is not secure.
fn fetch_url(url: &str) -> Result<Vec<u8>, RegistryError> {
    let program = if cfg!(windows) { "curl.exe" } else { "curl" };
    let mut command = Command::new(program);
    command.args(["--fail", "--silent", "--show-error", "--location"]);
    if url.starts_with("https://") {
        command.arg("--proto").arg("=https");
    }
    command.arg(url);
    let output = command.output().map_err(|error| {
        registry_error(format!(
            "cannot run `{program}` to fetch {url}: {error}. \
             A registry over HTTPS needs curl; a registry on this machine needs a path."
        ))
    })?;
    if !output.status.success() {
        let detail = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(registry_error(format!(
            "cannot fetch {url}: {}",
            if detail.is_empty() {
                "the request failed".to_string()
            } else {
                detail
            }
        )));
    }
    Ok(output.stdout)
}

/// The entry for one name and version, wherever the registry lives.
pub fn lookup_source(
    source: &RegistrySource,
    name: &str,
    version: &str,
) -> Result<Option<RegistryEntry>, RegistryError> {
    let mut entries = fetch_index(source)?;
    entries.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.version.cmp(&right.version))
    });
    Ok(entries
        .into_iter()
        .find(|entry| entry.name == name && entry.version == version))
}

/// The entries of a registry that match a query, wherever it lives.
pub fn search_source(
    source: &RegistrySource,
    query: Option<&str>,
) -> Result<Vec<RegistryEntry>, RegistryError> {
    let mut entries = fetch_index(source)?;
    entries.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.version.cmp(&right.version))
    });
    if let Some(query) = query {
        entries.retain(|entry| entry.name.contains(query) || entry.version.contains(query));
    }
    Ok(entries)
}

pub fn lookup(root: &Path, name: &str, version: &str) -> Result<Option<RegistryEntry>, RegistryError> {
    Ok(search(root, None)?
        .into_iter()
        .find(|entry| entry.name == name && entry.version == version))
}

fn tarball_path(root: &Path, entry: &RegistryEntry) -> PathBuf {
    root.join("packages").join(format!("{}.tar", entry.hash.trim_start_matches("sha256:")))
}

fn index_path(root: &Path) -> PathBuf {
    root.join("index.json")
}

pub fn publish(package_root: &Path, registry_root: &Path) -> Result<RegistryEntry, RegistryError> {
    let manifest = Manifest::load(&package_root.join("yashy.toml")).map_err(|error| {
        registry_error(format!(
            "yashy.toml:{}:{}: {}",
            error.line, error.column, error.message
        ))
    })?;
    let content_hash = hash_tree(package_root).map_err(|error| {
        registry_error(format!("cannot hash package tree: {error}"))
    })?;
    let hash = content_hash;
    let tarball = write_tarball(package_root)?;
    let entry = RegistryEntry {
        name: manifest.package.name.clone(),
        version: manifest.package.version.clone(),
        hash: hash.clone(),
        description: manifest.package.description.clone().unwrap_or_default(),
    };
    let mut entries = read_index(registry_root)?;
    if let Some(existing) = entries
        .iter()
        .find(|item| item.name == entry.name && item.version == entry.version)
    {
        if existing.hash != hash {
            return Err(registry_error(format!(
                "duplicate registry entry for `{}@{}`: index hash `{}` does not match package hash `{}`",
                entry.name, entry.version, existing.hash, hash
            )));
        }
        return Ok(existing.clone());
    }

    std::fs::create_dir_all(registry_root.join("packages"))
        .map_err(|error| registry_error(format!("cannot create registry: {error}")))?;
    let destination = tarball_path(registry_root, &entry);
    let staging = registry_root.join(format!(
        ".{}-{}.tmp",
        entry.hash.trim_start_matches("sha256:"),
        std::process::id()
    ));
    std::fs::write(&staging, tarball)
        .map_err(|error| registry_error(format!("cannot write registry tarball: {error}")))?;
    if let Err(error) = std::fs::rename(&staging, &destination) {
        let _ = std::fs::remove_file(&staging);
        if !destination.is_file() {
            return Err(registry_error(format!(
                "cannot install registry tarball {}: {error}",
                destination.display()
            )));
        }
    }

    entries.push(entry.clone());
    entries.sort_by(|left, right| {
        left.name
            .cmp(&right.name)
            .then_with(|| left.version.cmp(&right.version))
    });
    let staged_index = registry_root.join(format!(".index-{}.tmp", std::process::id()));
    std::fs::write(&staged_index, index_json(&entries))
        .map_err(|error| registry_error(format!("cannot write registry index: {error}")))?;
    if let Err(error) = std::fs::rename(&staged_index, index_path(registry_root)) {
        let _ = std::fs::remove_file(&staged_index);
        return Err(registry_error(format!(
            "cannot install registry index: {error}"
        )));
    }
    Ok(entry)
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TarEntry {
    pub path: String,
    pub bytes: Vec<u8>,
    pub directory: bool,
}

fn collect_tarball_entries(root: &Path) -> Result<Vec<TarEntry>, RegistryError> {
    let mut entries = Vec::new();
    collect_into(root, root, &mut entries)?;
    entries.sort_by(|left, right| left.path.cmp(&right.path));
    Ok(entries)
}

fn collect_into(
    root: &Path,
    directory: &Path,
    entries: &mut Vec<TarEntry>,
) -> Result<(), RegistryError> {
    let read = std::fs::read_dir(directory)
        .map_err(|error| registry_error(format!("cannot list {}: {error}", directory.display())))?;
    let mut paths: Vec<PathBuf> = read
        .map(|entry| entry.map(|entry| entry.path()))
        .collect::<Result<Vec<_>, _>>()
        .map_err(|error| registry_error(format!("cannot list {}: {error}", directory.display())))?;
    paths.sort();
    for path in paths {
        let relative = path
            .strip_prefix(root)
            .unwrap_or(&path)
            .to_string_lossy()
            .replace('\\', "/");
        if relative == ".git" || relative.starts_with(".git/") {
            continue;
        }
        let metadata = std::fs::symlink_metadata(&path)
            .map_err(|error| registry_error(format!("cannot read {}: {error}", path.display())))?;
        if metadata.is_dir() {
            entries.push(TarEntry {
                path: format!("{relative}/"),
                bytes: Vec::new(),
                directory: true,
            });
            collect_into(root, &path, entries)?;
        } else {
            let bytes = std::fs::read(&path)
                .map_err(|error| registry_error(format!("cannot read {}: {error}", path.display())))?;
            entries.push(TarEntry {
                path: relative,
                bytes,
                directory: false,
            });
        }
    }
    Ok(())
}

fn octal(bytes: &mut [u8], value: u64) {
    let width = bytes.len() - 1;
    let mut text = format!("{:0width$o}", value, width = width);
    text.push('\0');
    bytes.copy_from_slice(text.as_bytes());
}

fn octal_value(bytes: &[u8]) -> u64 {
    let text = String::from_utf8_lossy(bytes);
    u64::from_str_radix(text.trim_matches('\0').trim(), 8).unwrap_or(0)
}

fn tar_name(path: &str) -> (Vec<u8>, Vec<u8>) {
    if path.len() <= 100 {
        return (path.as_bytes().to_vec(), Vec::new());
    }
    if let Some(index) = path.rfind('/') {
        if index <= 155 && path.len() - index - 1 <= 100 {
            return (path[index + 1..].as_bytes().to_vec(), path[..index].as_bytes().to_vec());
        }
    }
    (path.as_bytes().to_vec(), Vec::new())
}

pub fn write_tarball(root: &Path) -> Result<Vec<u8>, RegistryError> {
    let entries = collect_tarball_entries(root)?;
    let mut out = Vec::new();
    for entry in &entries {
        let (name, prefix) = tar_name(&entry.path);
        if name.len() > 100 || prefix.len() > 155 {
            return Err(registry_error(format!(
                "registry tarball path is too long for ustar: {}",
                entry.path
            )));
        }
        let mut header = [0u8; 512];
        header[..name.len()].copy_from_slice(&name);
        octal(&mut header[100..108], 0o644);
        octal(&mut header[108..116], 0);
        octal(&mut header[116..124], 0);
        octal(&mut header[124..136], entry.bytes.len() as u64);
        octal(&mut header[136..148], 0);
        header[156] = if entry.directory { b'5' } else { b'0' };
        header[257..263].copy_from_slice(b"ustar\0");
        header[263..265].copy_from_slice(b"00");
        if !prefix.is_empty() {
            header[345..345 + prefix.len()].copy_from_slice(&prefix);
        }
        for slot in &mut header[148..156] {
            *slot = b' ';
        }
        let checksum: u32 = header.iter().map(|byte| u32::from(*byte)).sum();
        octal(&mut header[148..156], u64::from(checksum));
        for slot in &mut header[148..155] {
            if *slot == 0 {
                *slot = b'0';
            }
        }
        out.extend_from_slice(&header);
        out.extend_from_slice(&entry.bytes);
        let padding = (512 - (entry.bytes.len() % 512)) % 512;
        out.extend(std::iter::repeat(0u8).take(padding));
    }
    out.extend(std::iter::repeat(0u8).take(1024));
    Ok(out)
}

pub fn read_tarball(bytes: &[u8]) -> Result<Vec<TarEntry>, RegistryError> {
    let mut entries = Vec::new();
    let mut offset = 0usize;
    while offset + 512 <= bytes.len() {
        let mut header = bytes[offset..offset + 512].to_vec();
        if header.iter().all(|byte| *byte == 0) {
            break;
        }
        let expected = octal_value(&header[148..156]);
        for slot in &mut header[148..156] {
            *slot = b' ';
        }
        let checksum = header.iter().map(|byte| u32::from(*byte)).sum::<u32>();
        if u64::from(checksum) != expected {
            return Err(registry_error("registry tarball has a bad ustar checksum"));
        }
        if &header[257..263] != b"ustar\0" {
            return Err(registry_error("registry tarball is not ustar"));
        }
        let name = String::from_utf8_lossy(&header[..100])
            .trim_matches('\0')
            .to_string();
        let prefix = String::from_utf8_lossy(&header[345..500])
            .trim_matches('\0')
            .to_string();
        let path = if prefix.is_empty() {
            name
        } else {
            format!("{prefix}/{name}")
        };
        if path.is_empty() || path.starts_with('/') || path.split('/').any(|part| part == "..") {
            return Err(registry_error(format!("unsafe registry tarball path: {path}")));
        }
        let size = octal_value(&header[124..136]) as usize;
        let data_start = offset + 512;
        if data_start + size > bytes.len() {
            return Err(registry_error("registry tarball is truncated"));
        }
        entries.push(TarEntry {
            path: path.clone(),
            bytes: bytes[data_start..data_start + size].to_vec(),
            directory: header[156] == b'5',
        });
        let padding = (512 - (size % 512)) % 512;
        offset = data_start + size + padding;
    }
    Ok(entries)
}

pub fn extract_tarball(root: &Path, bytes: &[u8]) -> Result<(), RegistryError> {
    let entries = read_tarball(bytes)?;
    for entry in entries {
        let path = root.join(entry.path.trim_end_matches('/'));
        if entry.directory {
            std::fs::create_dir_all(&path)
                .map_err(|error| registry_error(format!("cannot create {}: {error}", path.display())))?;
        } else {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| registry_error(format!("cannot create {}: {error}", parent.display())))?;
            }
            std::fs::write(&path, entry.bytes)
                .map_err(|error| registry_error(format!("cannot write {}: {error}", path.display())))?;
        }
    }
    Ok(())
}

struct Parser<'a> {
    text: &'a str,
    index: usize,
}

impl<'a> Parser<'a> {
    fn new(text: &'a str) -> Parser<'a> {
        Parser { text, index: 0 }
    }

    fn remaining(&self) -> &'a str {
        &self.text[self.index..]
    }

    fn skip_whitespace(&mut self) {
        while self.remaining().starts_with(|character: char| character.is_whitespace()) {
            self.index += 1;
        }
    }

    fn expect_char(&mut self, wanted: char) -> Result<(), RegistryError> {
        self.skip_whitespace();
        if self.parse_char(wanted) {
            Ok(())
        } else {
            Err(registry_error(format!("expected `{wanted}` in registry index")))
        }
    }

    fn parse_char(&mut self, wanted: char) -> bool {
        self.skip_whitespace();
        let mut chars = self.remaining().chars();
        if chars.next() == Some(wanted) {
            self.index += wanted.len_utf8();
            true
        } else {
            false
        }
    }

    fn parse_string(&mut self) -> Result<String, RegistryError> {
        self.skip_whitespace();
        let mut chars = self.remaining().chars();
        if chars.next() != Some('"') {
            return Err(registry_error("expected a string in registry index"));
        }
        self.index += 1;
        let mut out = String::new();
        loop {
            let character = self
                .remaining()
                .chars()
                .next()
                .ok_or_else(|| registry_error("unterminated string in registry index"))?;
            self.index += character.len_utf8();
            match character {
                '"' => break,
                '\\' => {
                    let escape = self
                        .remaining()
                        .chars()
                        .next()
                        .ok_or_else(|| registry_error("unterminated escape in registry index"))?;
                    self.index += escape.len_utf8();
                    match escape {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'b' => out.push('\u{08}'),
                        'f' => out.push('\u{0c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => {
                            let hex = self.remaining().chars().take(4).collect::<String>();
                            if hex.len() != 4 || u16::from_str_radix(&hex, 16).is_err() {
                                return Err(registry_error("bad unicode escape in registry index"));
                            }
                            self.index += hex.len();
                            if let Some(character) =
                                u32::from_str_radix(&hex, 16).ok().and_then(char::from_u32)
                            {
                                out.push(character);
                            }
                        }
                        _ => return Err(registry_error("bad escape in registry index")),
                    }
                }
                character => out.push(character),
            }
        }
        Ok(out)
    }

    fn skip_value(&mut self) -> Result<(), RegistryError> {
        self.skip_whitespace();
        match self.remaining().chars().next() {
            Some('"') => {
                self.parse_string()?;
            }
            Some('{') => {
                self.expect_char('{')?;
                loop {
                    self.skip_whitespace();
                    if self.parse_char('}') {
                        break;
                    }
                    self.parse_string()?;
                    self.expect_char(':')?;
                    self.skip_value()?;
                    if !self.parse_char(',') {
                        break;
                    }
                }
            }
            Some('[') => {
                self.expect_char('[')?;
                loop {
                    self.skip_whitespace();
                    if self.parse_char(']') {
                        break;
                    }
                    self.skip_value()?;
                    if !self.parse_char(',') {
                        break;
                    }
                }
            }
            _ => {
                while let Some(character) = self.remaining().chars().next() {
                    if character.is_whitespace() || matches!(character, ',' | '}' | ']') {
                        break;
                    }
                    self.index += character.len_utf8();
                }
            }
        }
        Ok(())
    }
}

pub fn read_index_text(text: &str) -> Result<Vec<RegistryEntry>, RegistryError> {
    let mut parser = Parser::new(text);
    parser.expect_char('{')?;
    let mut found = false;
    let mut entries = Vec::new();
    loop {
        parser.skip_whitespace();
        if parser.parse_char('}') {
            break;
        }
        let key = parser.parse_string()?;
        parser.expect_char(':')?;
        if key == "packages" {
            found = true;
            parser.expect_char('[')?;
            loop {
                parser.skip_whitespace();
                if parser.parse_char(']') {
                    break;
                }
                parser.expect_char('{')?;
                let mut name = String::new();
                let mut version = String::new();
                let mut hash = String::new();
                let mut description = String::new();
                loop {
                    parser.skip_whitespace();
                    if parser.parse_char('}') {
                        break;
                    }
                    let field = parser.parse_string()?;
                    parser.expect_char(':')?;
                    let value = parser.parse_string()?;
                    match field.as_str() {
                        "name" => name = value,
                        "version" => version = value,
                        "hash" => hash = value,
                        "description" => description = value,
                        _ => {}
                    }
                    if !parser.parse_char(',') {
                        parser.expect_char('}')?;
                        break;
                    }
                }
                if name.is_empty() || version.is_empty() || hash.is_empty() {
                    return Err(registry_error(
                        "registry index entry is missing name, version, or hash",
                    ));
                }
                if entries.iter().any(|entry: &RegistryEntry| {
                    entry.name == name && entry.version == version
                }) {
                    return Err(registry_error(format!(
                        "duplicate registry entry for `{}@{}`",
                        name, version
                    )));
                }
                entries.push(RegistryEntry {
                    name,
                    version,
                    hash,
                    description,
                });
                if !parser.parse_char(',') {
                    parser.skip_whitespace();
                    if parser.parse_char(']') {
                        break;
                    }
                }
            }
        } else {
            parser.skip_value()?;
        }
        if !parser.parse_char(',') {
            break;
        }
    }
    parser.expect_char('}')?;
    if !found {
        return Err(registry_error("registry index has no packages array"));
    }
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tarball_round_trips() {
        let root = std::env::temp_dir().join(format!("yashy-tar-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(root.join("yashy.toml"), "[package]\n").unwrap();
        std::fs::write(root.join("src/lib.ya"), "pub fn x() {}\n").unwrap();
        let tarball = write_tarball(&root).unwrap();
        assert!(tarball.len() >= 1024);
        let entries = read_tarball(&tarball).unwrap();
        assert!(entries.iter().any(|entry| entry.path == "src/lib.ya"));
        let out = root.with_file_name(format!("{}-out", root.file_name().unwrap().to_string_lossy()));
        extract_tarball(&out, &tarball).unwrap();
        assert_eq!(
            hash_tree(&out).unwrap(),
            hash_tree(&root).unwrap()
        );
        let _ = std::fs::remove_dir_all(&out);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_duplicate_registry_entry_is_an_error() {
        let text = "{\"packages\":[{\"name\":\"lib\",\"version\":\"1.0.0\",\"hash\":\"sha256:a\",\"description\":\"\"},{\"name\":\"lib\",\"version\":\"1.0.0\",\"hash\":\"sha256:b\",\"description\":\"\"}]}";
        let error = read_index_text(text).unwrap_err();
        assert!(error.message.contains("duplicate registry entry for `lib@1.0.0`"), "{error}");
    }
}
