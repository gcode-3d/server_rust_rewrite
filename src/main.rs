use std::sync::mpsc;

use async_std::{
    fs::OpenOptions,
    task::{block_on, spawn_blocking},
};
use sqlx::{Connection, Error, Executor, Row, SqliteConnection};

#[macro_use]
extern crate nickel;

mod api_manager;
mod connection_manager;

fn main() {
    let api = api_manager::ApiManager::new();

    block_on(create_tables());

    let threads = spawn_blocking(move || {
        let api = block_on(api);

        api.server.listen("0.0.0.0:8000").unwrap();
    })

    block_on(threads);
}

async fn create_tables() {
    let _result = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open("storage.db")
        .await;

    let mut connection = (SqliteConnection::connect("storage.db")).await.unwrap();
    connection
        .execute(
            "
    
        CREATE TABLE IF NOT EXISTS settings (
            name VARCHAR(255) NOT NULL primary key,
            value VARCHAR(255)
        );
        CREATE TABLE IF NOT EXISTS users (
            username VARCHAR(255) NOT NULL primary key,
            password VARCHAR(255) NOT NULL,
            permissions INTEGER NOT NULL
        );
        CREATE TABLE IF NOT EXISTS tokens (
            username VARCHAR(255) NOT NULL,
            token VARCHAR(255) NOT NULL primary key,
            expire DATETIME,
            FOREIGN KEY(username) REFERENCES users(username) on update cascade on delete cascade
        );
    ",
        )
        .await
        .expect("Error while creating tables.");

    let x = connection.fetch_all("select * from tokens").await.unwrap();
    for y in x {
        println!(
            "{}: {}",
            (y.try_get("username") as Result<String, Error>).unwrap(),
            (y.try_get("token") as Result<String, Error>).unwrap()
        );
    }
}
