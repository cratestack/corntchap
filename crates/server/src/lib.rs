#![allow(clippy::too_many_arguments, clippy::type_complexity)]
pub mod activity;
pub mod admin;
pub mod app;
pub mod auth;
pub mod cloud;
pub mod coder;
pub mod connect;
pub mod console;
pub mod devin;
pub mod error;
pub mod files;
pub mod forge;
pub mod jobs;
pub mod mcp_compat;
pub mod menus;
pub mod oauth;
pub mod pages;
pub mod pantry;
pub mod pass;
pub mod procs;
pub mod questions;
pub mod repos;
pub mod rest;
pub mod service;
pub mod sessions;
pub mod state;
pub mod stations;
pub mod stream;
pub mod usage;
pub mod views;
pub mod web;

pub mod schema {
    use cratestack::include_server_schema;
    include_server_schema!("schema/corntchap.cstack", db = Postgres);
}
