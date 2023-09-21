//! Madara node command line.
#![warn(missing_docs)]

#[macro_use]
mod service;
mod benchmarking;
mod chain_spec;
mod cli;
mod command;
mod genesis_block;
mod rpc;
mod starknet;

use std::thread;

use mc_sync_block;

fn main() -> sc_cli::Result<()> {
    thread::spawn(move || {
        mc_sync_block::sync_with_da();
    });
    command::run()
}
