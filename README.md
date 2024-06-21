# wiki-extractor

[![CI Status](https://img.shields.io/github/actions/workflow/status/Caellian/wiki-extractor/build.yml?style=for-the-badge&logo=githubactions&logoColor=%23fff&label=CI)](https://github.com/Caellian/wiki-extractor/actions/workflows/build.yml)
[![GPLv3 license](https://img.shields.io/crates/l/contiguous-mem?style=for-the-badge)](https://github.com/Caellian/wiki-extractor#license)

A tool that turns Wikipedia dump files into easily consumable data.

There's already a few tools such as this available in the wild, but none of them
properly support the dump XML format nor Mediawiki format and instead work
around them.

None of the existing tools utilize the fact that dumps are stored in gzip2
format which can be streamed either. This tool exploits that in order to allow
directly streaming Wikipedia dumps into processed target files,
**without storing intermediate files** which take up a lot of space (>20 GiB;
compressed).

Lastly, most existing tools don't actually parse Mediawiki format and instead
use finite-state automata (i.e. regex) in order to scrub files from Mediawiki
formatting and XML tags. This is incredibly faulty and produces very noisy
results, as well as being **much slower** than correctly parsing the articles.
Badly scrapped data negatively affects the quality and size of trained models,
making them both larger and more error-prone.

## Features

- Streams dump information directly from mirrors, without requiring the user to
  download dumps up-front.
- Decompresses the stream automatically without requiring external tools for
  extraction.
- Produces multiple useful outputs at once:
  - Sentence/text dump
  - Dictionary
  - Article metadata (WIP)
  - List of page redirections
- Can produce Markdown format if want to train a model on that instead.
- Partial output is still usable as articles are processed one-by-one in
  sequence.
  - Mirrors prefer you don't parallel download.

See the [Samples](./Samples.md) file for a more in-depth overview of produced
files/output.

## Usage

Download latest stable binary from [Releases](https://github.com/Caellian/wiki-extractor/releases).

### Configuration

All configuration is done through CLI arguments.

Print arguments with:
```sh
wiki-extractor --help
wiki-extractor local --help
wiki-extractor remote --help
```

### Running

- Find a closest [mirror](https://dumps.wikimedia.org/mirrors.html) to make the download faster.
  - Used URL should be the part before `/enwiki/latest` part once you locate the dump files.
- Replace the URL in command below with your mirror of choice:

```sh
wiki-extractor remote https://dumps.wikimedia.org/ -L en -w latest -o 
```

- Kick back and relax.

## Contributing

Contributions are very welcome. There's several `TODO` and `FIXME` comments in
the code that might offer ideas for wanted/high priority contributions, and you
can also just contribute what you personally need.

## License

This tool is licensed under GPLv3 license. A copy of the license is available in
the [`LICENSE`](./LICENSE) file.

This license only applies to the provided source code and binaries built with it
and doesn't apply to generated dump files. Produced dump files are licensed
under the same license as Wikipedia content -
[CC-BY-SA](https://en.wikipedia.org/wiki/Wikipedia:CCBYSA).
