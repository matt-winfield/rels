# Rels - Simple CLI tool for tracking releases via Git

This is a CLI tool aiming to simplify tracking which JIRA tickets are in which releases.

By default, it assumes each tag in the repository is a release.

## Installation

Requires Rust / Cargo - Install at https://rustup.rs/

Build + install using `cargo install rels`.

## Usage

To view all releases, and all the tickets in each release, simply run `rels` from within the Git repo.

See `rels --help` for other available commands.
