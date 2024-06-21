#![allow(incomplete_features)]
#![feature(adt_const_params)]

use clap::Parser;
use env_logger::Env;
use quick_xml::Reader as XMLReader;
use reqwest::Client;

use crate::{
    dump_data::DocumentContext,
    input::data::DumpInfo,
    output::DataGenerator,
    state::{set_tracker_global, DownloadTracker},
    xml_util::HandleEvent,
};

mod dump_data;
mod format;
mod input;
mod output;
mod state;
mod xml_util;

pub fn client() -> Client {
    static APP_USER_AGENT: &str = concat![
        env!("CARGO_PKG_NAME"),
        "/",
        env!("CARGO_PKG_VERSION"),
        " (github.com/Caellian/wiki-extractor)"
    ];

    reqwest::Client::builder()
        .user_agent(APP_USER_AGENT)
        .build()
        .expect("unable to create app web client")
}

#[derive(Parser)]
#[command(version, about)]
#[command(name = env!("CARGO_PKG_NAME"))]
#[command(about = env!("CARGO_PKG_DESCRIPTION"))]
pub struct Args {
    /// Input mirror/file.
    #[clap(subcommand)]
    pub input: input::data::SourceLocation,
    /// Path to output directory.
    #[arg(short = 'o', long = "output", default_value = "./dump")]
    pub output: std::path::PathBuf,

    /// Selection of generated files.
    #[clap(flatten)]
    pub generator: output::options::GeneratorOptions,
    /// Options for text dump generation.
    #[clap(flatten)]
    pub text: output::options::TextOptions,
}

fn main() -> anyhow::Result<()> {
    env_logger::Builder::from_env(Env::default().default_filter_or("info"))
        .format(crate::format::format)
        .init();

    let Args {
        input,
        output,
        generator: generator_options,
        text: text_options,
    } = Args::parse();

    if !generator_options.any() {
        log::info!("Nothing to do. See `--help` for list of generators.");
        std::process::exit(0);
    }

    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .unwrap();

    let dump = DumpInfo::new(rt.handle(), &input);

    if dump.status.map(|it| it != "done").unwrap_or_default() {
        log::error!("mirror is currently generating the dump; specify older version or wait");
        std::process::exit(1);
    }

    let mut gen = DataGenerator::new(output, generator_options, text_options)?;

    if let Some(updated) = dump.updated {
        log::info!("Dump creation date: {updated}");
    }

    let mut dt = DownloadTracker::new(&dump.files);
    unsafe {
        // SAFETY: DownloadTracker is constructed once, and never moved.
        // Have to do it this way because logger is initialized before tracker.
        set_tracker_global(&dt)
    };
    log::info!(
        "Total download size: {:.3} GB",
        dt.total_size() as f32 / 1024. / 1024. / 1024.
    );

    // TODO: Allow user to continue as we know where we left off in the stream
    // and can easily serialize entire state.

    // Don't paralelize streaming because you'll get your IP address blocked and
    // it's very unpolite towards everyone else accessing the data.
    for (name, stats) in dump.files {
        log::info!("Handling {name}...");

        let data_size = stats.size;

        let stream = stats.path.stream(rt.handle())?;

        let mut xml_reader = XMLReader::from_reader(stream);
        let mut stream_buffer = Vec::new();
        let mut document = DocumentContext::new(&stats.path);

        while xml_reader.buffer_position() < data_size {
            dt.set_current_position(xml_reader.buffer_position());

            let event = xml_reader.read_event_into(&mut stream_buffer)?;
            if let Err(err) = document.handle_event(event) {
                log::error!("Error while reading {name}: {}", err.to_string());
                break;
            };

            let process_result = rt.block_on(gen.process_document(&mut document));
            
            stream_buffer.clear();
            if let Err(err) = process_result {
                log::error!("Error processing '{name}' document: {}", err);
                break;
            }
        }

        dt.advance_file();
    }
    log::info!("Done!");

    gen.finalize()?;
    Ok(())
}
