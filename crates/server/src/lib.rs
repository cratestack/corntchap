#![allow(clippy::too_many_arguments, clippy::type_complexity)]
pub mod admin;
pub mod app;
pub mod auth;
pub mod console;
pub mod error;
pub mod jobs;
pub mod mcp_compat;
pub mod pages;
pub mod pantry;
pub mod procs;
pub mod rest;
pub mod service;
pub mod sessions;
pub mod state;
pub mod stream;
pub mod web;

pub mod schema {
    use cratestack::include_server_schema;
    include_server_schema!("schema/corntchap.cstack", db = Postgres);
}
