use std::{
    collections::{BTreeMap, HashMap},
    fmt::Display,
    fs::File,
    io::{ErrorKind, Seek},
    path::{Path, PathBuf},
    str::FromStr,
};

use bytes::Bytes;
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tokio::runtime::Handle;
use url::Url;

use super::io::{CompressionAdapter, DocumentStream, SourceAdapter};
use crate::client;

static DUMP_STATUS_FILE: &str = "dumpstatus.json";

#[derive(Debug, Clone, Hash, PartialEq, Eq, Parser, Serialize, Deserialize)]
pub struct RemoteParams {
    /// Remote mirror file
    #[arg(name = "URL")]
    pub base: Url,
    /// Dump version (i.e. date) to download.
    #[arg(
        short = 'w',
        long = "dump-version",
        default_value_t = {"latest".to_string()},
    )]
    pub version: String,
    /// Wikipedia language.
    #[arg(
        short = 'L',
        long = "language",
        default_value_t = {"en".to_string()},
    )]
    pub language: String,
}

#[derive(Debug, Clone, Hash, PartialEq, Eq, Subcommand, Serialize, Deserialize)]
pub enum SourceLocation {
    /// Use remote dump file(s) as input.
    Remote {
        #[clap(flatten)]
        params: RemoteParams,
    },
    /// Use local dump file(s) as input.
    Local {
        /// Path to a dump file.
        #[arg(name = "PATH")]
        path: PathBuf,
    },
}

impl Default for SourceLocation {
    fn default() -> Self {
        SourceLocation::Remote {
            params: RemoteParams {
                base: Url::parse("https://dumps.wikimedia.org/").unwrap(),
                version: "latest".to_string(),
                language: "en".to_string(),
            },
        }
    }
}

impl Display for SourceLocation {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            SourceLocation::Remote {
                params:
                    RemoteParams {
                        base,
                        version,
                        language,
                    },
            } => f.write_fmt(format_args!(
                "{}/{}wiki/{}",
                base.as_str(),
                version,
                language
            )),
            SourceLocation::Local { path } => f.write_str(path.display().to_string().as_str()),
        }
    }
}

impl FromStr for SourceLocation {
    type Err = std::convert::Infallible;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        match Url::parse(s) {
            Ok(it) => Ok(SourceLocation::Remote {
                params: RemoteParams {
                    base: it,
                    version: "latest".to_string(),
                    language: "en".to_string(),
                },
            }),
            Err(_) => PathBuf::from_str(s).map(|path| SourceLocation::Local { path }),
        }
    }
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct DumpLocation {
    base: SourceLocation,
    file_name: FileName,
}

impl DumpLocation {
    #[inline(always)]
    pub fn name(&self) -> &FileName {
        &self.file_name
    }

    #[inline(always)]
    pub fn is_compressed(&self) -> bool {
        self.file_name.ext() == Some("bz2")
    }

    fn read_adapter(&self, rt: &Handle) -> std::io::Result<SourceAdapter> {
        Ok(match &self.base {
            SourceLocation::Local { path } => {
                let file = File::open(path)?;
                SourceAdapter::Local(std::io::BufReader::new(file))
            }
            SourceLocation::Remote { params } => {
                let file_url = format!(
                    "{}/{}wiki/{}/{}",
                    params.base, params.language, params.version, self.file_name
                );
                let file_response = rt.block_on(client().get(file_url).send()).map_err(|err| {
                    std::io::Error::new(std::io::ErrorKind::ConnectionRefused, err)
                })?;
                SourceAdapter::Remote {
                    resp: file_response,
                    buffer: Bytes::new(),
                    pos: 0,
                    runtime: rt.clone(),
                }
            }
        })
    }

    pub fn stream(&self, rt: &Handle) -> std::io::Result<DocumentStream> {
        let reader = self.read_adapter(rt)?;

        let reader = if self.is_compressed() {
            CompressionAdapter::new_bzip2(reader)
        } else {
            CompressionAdapter::new_passthrough(reader)
        };

        Ok(DocumentStream::new(reader))
    }
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
pub struct FileDescriptor {
    pub size: usize,
    pub path: DumpLocation,
    pub md5: Option<String>,
    pub sha1: Option<String>,
}

#[derive(Debug, Clone, Hash, Serialize, Deserialize)]
struct MirrorDumpEntry {
    pub size: usize,
    pub url: String,
    pub md5: Option<String>,
    pub sha1: Option<String>,
}
impl MirrorDumpEntry {
    fn to_descriptor(&self, source: &RemoteParams) -> FileDescriptor {
        // FIXME: Assumes files aren't nested; format allows them to be.
        let file_name = FileName(
            self.url
                .rsplit('/')
                .next()
                .expect("missing file name")
                .to_string(),
        );
        FileDescriptor {
            size: self.size,
            path: DumpLocation {
                base: SourceLocation::Remote {
                    params: source.clone(),
                },
                file_name,
            },
            md5: self.md5.clone(),
            sha1: self.sha1.clone(),
        }
    }
}

#[derive(Debug, Error)]
#[error("provided path does not point to a file: {provided}")]
pub struct NotAFile {
    provided: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
#[repr(transparent)]
pub struct FileName(String);

impl FileName {
    pub fn full_ext(&self) -> Option<&str> {
        self.0.find('.').map(|i| &self.0[(i + 1)..])
    }

    pub fn ext(&self) -> Option<&str> {
        self.0.rfind('.').map(|i| &self.0[(i + 1)..])
    }
}

impl TryFrom<&Path> for FileName {
    type Error = std::io::Error;

    fn try_from(path: &Path) -> std::io::Result<FileName> {
        Ok(FileName(
            String::from_utf8(
                path.file_name()
                    .ok_or_else(|| {
                        std::io::Error::new(
                            ErrorKind::InvalidData,
                            NotAFile {
                                provided: path.to_path_buf(),
                            },
                        )
                    })?
                    .as_encoded_bytes()
                    .to_vec(),
            )
            .map_err(|it| std::io::Error::new(ErrorKind::InvalidData, it.utf8_error()))?,
        ))
    }
}
impl TryFrom<PathBuf> for FileName {
    type Error = std::io::Error;

    fn try_from(path: PathBuf) -> std::io::Result<FileName> {
        FileName::try_from(path.as_path())
    }
}
impl TryFrom<&PathBuf> for FileName {
    type Error = std::io::Error;

    fn try_from(path: &PathBuf) -> std::io::Result<FileName> {
        FileName::try_from(path.as_path())
    }
}

impl Display for FileName {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.as_str())
    }
}

impl AsRef<str> for FileName {
    fn as_ref(&self) -> &str {
        self.0.as_str()
    }
}

impl Ord for FileName {
    #[inline(always)]
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        human_sort::compare(&self.0, &other.0)
    }
}

impl PartialOrd for FileName {
    #[inline(always)]
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Debug, Default, Clone, Deserialize)]
pub struct DumpInfo {
    pub status: Option<String>,
    pub updated: Option<String>,
    pub files: BTreeMap<FileName, FileDescriptor>,
}

impl DumpInfo {
    // TODO: Return errors
    async fn new_remote(params: &RemoteParams) -> DumpInfo {
        use serde_json::*;

        let RemoteParams {
            base: base_url,
            version,
            language,
        } = params;

        let file = format!(
            "{}/{}wiki/{}/{}",
            base_url, language, version, DUMP_STATUS_FILE
        );
        let dump_status_url = Url::parse(&file).expect("invalid dump status url format");

        let resp = match client().get(dump_status_url).send().await {
            Ok(it) => it,
            Err(_) => panic!("invalid dump status url"),
        };

        let dump_status = match resp.text().await {
            Ok(it) => it,
            Err(_) => panic!("invalid remote '{}' file", DUMP_STATUS_FILE),
        };

        // TODO: Cleanup
        let mut articlesdump: Map<String, Value> = match from_str::<Value>(&dump_status) {
            Ok(it) => match it {
                Value::Object(mut root) => {
                    let jobs = root
                        .remove("jobs")
                        .expect("unsupported 'dumpstatus.json' format");

                    let articlesdump = match jobs {
                        Value::Object(mut jobs) => jobs
                            .remove("articlesdump")
                            .expect("unsupported 'dumpstatus.json' format"),
                        _ => panic!("unsupported '{}' format", DUMP_STATUS_FILE),
                    };

                    match articlesdump {
                        Value::Object(it) => it,
                        _ => panic!("unsupported '{}' format", DUMP_STATUS_FILE),
                    }
                }
                _ => panic!("unsupported '{}' format", DUMP_STATUS_FILE),
            },
            Err(_) => panic!("dump remote URL doesn't have a supported JSON file"),
        };

        let file_list: HashMap<String, MirrorDumpEntry> = match articlesdump
            .remove("files")
            .and_then(|it| from_value(it).ok())
        {
            Some(value) => value,
            _ => panic!("unsupported '{}' format", DUMP_STATUS_FILE),
        };
        let status = articlesdump.remove("status").and_then(|it| match it {
            Value::String(it) => Some(it),
            _ => None,
        });
        let updated = articlesdump.remove("updated").and_then(|it| match it {
            Value::String(it) => Some(it),
            _ => None,
        });

        let mut files = BTreeMap::new();
        for (name, data) in file_list {
            let file_name = FileName(name);
            files.insert(file_name, data.to_descriptor(params));
        }

        DumpInfo {
            status,
            updated,
            files,
        }
    }

    // TODO: Return errors
    // TODO: Support split files
    pub fn new(rt: &Handle, source: &SourceLocation) -> DumpInfo {
        match source {
            SourceLocation::Local { path } => {
                let mut files = BTreeMap::<FileName, FileDescriptor>::new();

                let file_name = FileName::try_from(path).expect("non UTF-8 dump file name");
                let mut test_open = File::open(path).expect("unable to open dump file");
                let size = test_open
                    .seek(std::io::SeekFrom::End(0))
                    .expect("unable to read (seek) dump file") as usize;
                files.insert(
                    file_name.clone(),
                    FileDescriptor {
                        size,
                        path: DumpLocation {
                            base: SourceLocation::Local { path: path.clone() },
                            file_name,
                        },
                        md5: None,
                        sha1: None,
                    },
                );

                DumpInfo {
                    status: None,
                    updated: None,
                    files,
                }
            }
            SourceLocation::Remote { params } => rt.block_on(Self::new_remote(params)),
        }
    }
}
