use crate::config::Config;
use anyhow::Result;
use serde::Serialize;
use sqlx::{pool::PoolOptions, Executor, Pool, Postgres, Sqlite, SqliteConnection};

pub struct Db {
    pub abbs: Pool<Sqlite>,
    pub pg: Pool<Postgres>,
}

pub const PAGESIZE: u32 = 60;

impl Db {
    pub async fn open(config: &Config) -> Result<Self> {
        let opt = sqlx::sqlite::SqliteConnectOptions::new()
            .read_only(true)
            .immutable(true)
            .foreign_keys(false)
            .collation("vercomp", deb_version::compare_versions)
            .filename(&config.abbs)
            .journal_mode(sqlx::sqlite::SqliteJournalMode::Off);

        let attach_piss = Box::leak(
            format!(
                "ATTACH DATABASE 'file:{}?mode=ro&immutable=1' AS piss",
                config.piss
            )
            .into_boxed_str(),
        );

        let abbs: Pool<Sqlite> = PoolOptions::new()
            .after_connect(|conn: &mut SqliteConnection, _| {
                Box::pin(async {
                    let attach_piss = &*attach_piss;
                    conn.execute(attach_piss).await?;
                    Ok(())
                })
            })
            .connect_with(opt)
            .await?;

        let pg = PoolOptions::new().connect_lazy(&config.pg_conn)?;

        Ok(Db { abbs, pg })
    }
}

#[derive(Debug, Default, Serialize)]
pub struct Page {
    pub cur: u32,
    pub max: u32,
    pub count: u32,
}

macro_rules! get_page {
    ($sql:expr,$name:ident,$cur:expr,$db:expr,$($bind_value:expr),+ $(,)?) =>  {
        async {

            if let Some(cur) = $cur {
                use sqlx::Row;
                use crate::db::PAGESIZE;
                let sql = format!("SELECT COUNT(*) OVER (),*  FROM ({}) LIMIT ? OFFSET ?",$sql);
                let query = sqlx::query(&sql);
                $(
                    let query = query.bind($bind_value);
                )*
                let mut count:Option<u32> = None;
                let rows = query.bind(PAGESIZE).bind((cur - 1) * PAGESIZE).try_map(|ref row| {
                    count.get_or_insert(row.try_get(0)?);
                    $name::from_row(row)
                }).fetch_all($db).await;

                let count = count.unwrap_or(0);
                let ceil = |a,b| (a + b - 1) / b;
                match rows {
                    Ok(rows) => {
                        let page = crate::db::Page {
                            cur,
                            max:ceil(count,PAGESIZE),
                            count,
                        };
                        Ok((page,rows))},
                    Err(e) => Err(e),
                }
            }else{
                let query = sqlx::query_as($sql);
                $(
                    let query = query.bind($bind_value);
                )*

                let rows:Vec<$name> = query.fetch_all($db).await?;

                let page = crate::db::Page {
                    cur:1,
                    max:0,
                    count:rows.len() as u32,
                };

                Ok((page,rows))
            }

        }
    }
}

pub(crate) use get_page;
