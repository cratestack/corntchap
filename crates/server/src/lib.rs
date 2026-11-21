pub mod admin;
pub mod app;
pub mod auth;
pub mod error;
pub mod jobs;
pub mod procs;
pub mod rest;
pub mod service;
pub mod state;
pub mod stream;
pub mod web;

pub mod schema {
    use cratestack::include_server_schema;
    include_server_schema!("schema/corntchap.cstack", db = Postgres);
}
