use crate::config::Config;
use anyhow::Result;
use axum::async_trait;
use itertools::Itertools;
use serde::Serialize;
use sqlx::{pool::PoolOptions, query::QueryAs, Database, Executor, FromRow, IntoArguments, Pool, Postgres, Sqlite};

pub struct Db {
    pub abbs: Pool<Sqlite>,
    pub pg: Pool<Postgres>,
}

const PAGESIZE: u32 = 60;

impl Db {
    pub async fn open(config: &Config) -> Result<Self> {
        let opt = sqlx::sqlite::SqliteConnectOptions::new()
            .read_only(true)
            .immutable(true)
            .foreign_keys(false)
            .collation("vercomp", deb_version::compare_versions)
            .filename(&config.db.abbs)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Off);

        let abbs: Pool<Sqlite> = PoolOptions::new().connect_with(opt).await?;
        let pg = PoolOptions::new().connect_lazy(&config.db.pg_conn)?;

        Ok(Db { abbs, pg })
    }
}

#[derive(Debug, Default, Serialize)]
pub struct Page {
    pub cur: u32,
    pub max: u32,
    pub count: u32,
}

#[async_trait]
pub trait Paginator<'q, DB, O, A>
where
    DB: Database,
    A: 'q + IntoArguments<'q, DB>,
    O: Send + Unpin + for<'r> FromRow<'r, DB::Row>,
{
    async fn fetch_page<'e, 'c: 'e, E>(self, executor: E, cur: Option<u32>) -> Result<(Vec<O>, Page), sqlx::Error>
    where
        'q: 'e,
        Self: Sized,
        E: 'e + Executor<'c, Database = DB>,
        DB: 'e,
        O: 'e,
        A: 'e;
}

#[async_trait]
impl<'q, DB, O, A> Paginator<'q, DB, O, A> for QueryAs<'q, DB, O, A>
where
    DB: Database,
    A: 'q + IntoArguments<'q, DB>,
    O: Send + Unpin + for<'r> FromRow<'r, DB::Row>,
{
    async fn fetch_page<'e, 'c: 'e, E>(mut self, executor: E, cur: Option<u32>) -> Result<(Vec<O>, Page), sqlx::Error>
    where
        'q: 'e,
        Self: Sized,
        E: 'e + Executor<'c, Database = DB>,
        DB: 'e,
        O: 'e,
        A: 'e,
    {
        let v = self.fetch_all(executor).await?;
        let count = v.len() as u32;
        let ceil = |a, b| (a + b - 1) / b;

        let (res, page) = if let Some(cur) = cur {
            let res = v
                .into_iter()
                .chunks(PAGESIZE as usize)
                .into_iter()
                .nth((cur - 1) as usize)
                .map_or(vec![], |i| i.collect_vec());
            let max = ceil(count, PAGESIZE);

            (res, Page { cur, max, count })
        } else {
            (v, Page { cur: 1, max: 0, count })
        };

        Ok((res, page))
    }
}
