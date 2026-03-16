![Build](https://github.com/fulcrumgenomics/refget-rs/actions/workflows/check.yml/badge.svg)
[![Version at crates.io](https://img.shields.io/crates/v/refget-server)](https://crates.io/crates/refget-server)
[![Documentation at docs.rs](https://img.shields.io/docsrs/refget-server)](https://docs.rs/refget-server)
[![codecov](https://codecov.io/gh/fulcrumgenomics/refget-rs/graph/badge.svg)](https://codecov.io/gh/fulcrumgenomics/refget-rs)
[![License](http://img.shields.io/badge/license-MIT-blue.svg)](https://github.com/fulcrumgenomics/refget-rs/blob/main/LICENSE)

# refget-rs

A Rust implementation of the [GA4GH refget](https://samtools.github.io/hts-specs/refget.html) API, covering both the **Sequences v2.0.0** and **Sequence Collections v1.0.0** specifications.

<p>
<a href="https://fulcrumgenomics.com"><img src="https://raw.githubusercontent.com/fulcrumgenomics/fgumi/main/.github/logos/fulcrumgenomics.svg" alt="Fulcrum Genomics" height="100"/></a>
</p>

<a href="mailto:contact@fulcrumgenomics.com?subject=[GitHub inquiry]"><img src="https://img.shields.io/badge/Email_us-brightgreen.svg?&style=for-the-badge&logo=gmail&logoColor=white"/></a>
<a href="https://www.fulcrumgenomics.com"><img src="https://img.shields.io/badge/Visit_Us-blue.svg?&style=for-the-badge&logo=wordpress&logoColor=white"/></a>

## Overview

refget-rs provides:

- **`refget-digest`** — SHA-512/24 (`sha512t24u`) and RFC 8785 JSON Canonicalization (JCS) computation
- **`refget-model`** — Domain types for sequences and sequence collections, including the comparison algorithm
- **`refget-store`** — Storage traits with in-memory and indexed-FASTA backends
- **`refget-server`** — Composable [Axum](https://github.com/tokio-rs/axum) router library for all refget endpoints
- **`refget-server-bin`** — Standalone server binary that serves FASTA files via the refget API
- **`refget-tools`** — CLI tool for computing refget digests from FASTA files

## Installation

### Building from source

Clone the repository:

```bash
git clone https://github.com/fulcrumgenomics/refget-rs
```

Build the release binaries:

```bash
cd refget-rs
cargo build --release
```

The two binaries will be at `target/release/refget-server` and `target/release/refget-tools`.

## Usage

### Serving FASTA files

Start a refget server that serves one or more indexed FASTA files:

```bash
refget-server --fasta ref.fa --port 8080
```

You can pass multiple files or a directory:

```bash
refget-server --fasta /path/to/fastas/ --port 8080
```

**Note:** Each FASTA file must have a companion `.fai` index (e.g. `ref.fa.fai`).

### API Endpoints

#### Sequences v2.0.0

| Endpoint | Description |
|---|---|
| `GET /sequence/service-info` | Service info (algorithms, formats) |
| `GET /sequence/{digest}` | Retrieve sequence bases (supports `Range` header, `start`/`end` query params) |
| `GET /sequence/{digest}/metadata` | Sequence metadata (MD5, sha512t24u, length, aliases) |

#### Sequence Collections v1.0.0

| Endpoint | Description |
|---|---|
| `GET /service-info` | Service info with collection schema |
| `GET /collection/{digest}?level=0\|1\|2` | Collection at specified level (default: 2) |
| `GET /comparison/{digest1}/{digest2}` | Compare two collections |
| `POST /comparison/{digest1}` | Compare a collection against POSTed Level 2 JSON |
| `GET /list/collection` | Paginated collection listing with attribute filters |
| `GET /attribute/collection/{attr}/{digest}` | Single attribute array by digest |

### Computing digests

Compute per-sequence digests from a FASTA file:

```bash
refget-tools digest-fasta ref.fa
```

Output (TSV): `name`, `length`, `md5`, `sha512t24u`

Compute sequence collection digests:

```bash
refget-tools digest-collection ref.fa
```

## Resources

* [GA4GH refget specification](https://samtools.github.io/hts-specs/refget.html)
* [Sequence Collections specification](https://ga4gh.github.io/seqcol-spec/)
* [Issues](https://github.com/fulcrumgenomics/refget-rs/issues): Report a bug or request a feature
* [Pull requests](https://github.com/fulcrumgenomics/refget-rs/pulls): Submit a patch or new feature
* [Contributors guide](https://github.com/fulcrumgenomics/refget-rs/blob/main/CONTRIBUTING.md)
* [License](https://github.com/fulcrumgenomics/refget-rs/blob/main/LICENSE): Released under the MIT license

## Authors

- [Nils Homer](https://github.com/nh13)

## Sponsors

Development of refget-rs is supported by [Fulcrum Genomics](https://www.fulcrumgenomics.com).

[Become a sponsor](https://github.com/sponsors/fulcrumgenomics)

## Disclaimer

This software is under active development.
While we make a best effort to test this software and to fix issues as they are reported, this software is provided as-is without any warranty (see the [license](https://github.com/fulcrumgenomics/refget-rs/blob/main/LICENSE) for details).
Please submit an [issue](https://github.com/fulcrumgenomics/refget-rs/issues), and better yet a [pull request](https://github.com/fulcrumgenomics/refget-rs/pulls) as well, if you discover a bug or identify a missing feature.
Please contact [Fulcrum Genomics](https://www.fulcrumgenomics.com) if you are considering using this software or are interested in sponsoring its development.
