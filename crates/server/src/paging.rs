//! Keyset paging for the console's `*Page` procedures and a few REST lists.
//!
//! A cursor is cratestack's own (`encode_cursor`): base64url JSON carrying the sort it was minted
//! for and the last row's key values as text. A cursor minted for another sort (another bucket, status
//! or view) is refused with a 400. Lists fetch `limit + 1` rows and [`finish`] turns them into a page.

use cratestack::{CratestackError, CursorPage, CursorPageInput, decode_cursor, encode_cursor};
use serde_json::Value;

use crate::error::{Error, Result};

/// Key values of one row, as text (`None` is NULL), in sort order.
pub type Keys = Vec<Option<String>>;

pub const DEFAULT_LIMIT: i64 = 25;
pub const MAX_LIMIT: i64 = 200;

/// What a page query needs: how many rows, where to start after, and the sort the cursor is for.
#[derive(Debug, Clone)]
pub struct Paging {
    pub limit: i64,
    pub after: Option<Keys>,
    pub order: String,
}

impl Paging {
    /// `limit + 1`: the extra row says whether there is a next page.
    pub fn fetch(&self) -> i64 {
        self.limit + 1
    }

    /// The `n`th key of the cursor, or `None` on the first page.
    pub fn key(&self, n: usize) -> Option<String> {
        self.after
            .as_ref()
            .and_then(|k| k.get(n).cloned().flatten())
    }
}

fn bad_cursor(e: CratestackError) -> Error {
    match e {
        CratestackError::BadRequest(m) => Error::Invalid(m),
        other => Error::Internal(other.to_string()),
    }
}

/// The page request of a `*Page` procedure for the sort `order` (`nkeys` key columns). The limit
/// defaults to `default` and is clamped into `1..=max`.
pub fn paging(
    page: &CursorPageInput,
    order: &str,
    nkeys: usize,
    default: i64,
    max: i64,
) -> Result<Paging> {
    paging_raw(
        page.limit,
        page.cursor.as_deref(),
        order,
        nkeys,
        default,
        max,
    )
}

pub fn paging_raw(
    limit: Option<i64>,
    cursor: Option<&str>,
    order: &str,
    nkeys: usize,
    default: i64,
    max: i64,
) -> Result<Paging> {
    let after = match cursor.filter(|c| !c.is_empty()) {
        Some(c) => Some(decode_cursor(c, order, nkeys).map_err(bad_cursor)?),
        None => None,
    };
    Ok(Paging {
        limit: limit.unwrap_or(default).clamp(1, max),
        after,
        order: order.to_string(),
    })
}

/// The cursor for the page that starts after a row with these keys.
pub fn cursor(order: &str, keys: &Keys) -> String {
    encode_cursor(order, keys)
}

/// One page: the rows fetched with `limit + 1` (each with its keys) cut to `limit`, and the cursor
/// of the next page when there are more.
pub fn finish(rows: Vec<(Value, Keys)>, p: &Paging) -> (Vec<Value>, Option<String>) {
    let page = CursorPage::from_rows(rows, p.limit, |r| encode_cursor(&p.order, &r.1));
    (
        page.items.into_iter().map(|r| r.0).collect(),
        page.next_cursor,
    )
}

/// A `CursorPage` as the JSON the typed procedure output reads.
pub fn page_json(items: Vec<Value>, next: Option<String>) -> Value {
    let mut o = serde_json::json!({ "items": items });
    if let Some(n) = next {
        o["nextCursor"] = Value::String(n);
    }
    o
}
