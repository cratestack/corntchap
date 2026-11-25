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

/// A cursor is client-held text: its key values reach SQL as `::text::bigint` / `::text::timestamptz`
/// casts, and a value that does not fit would fail there as a 500. Each key is checked against the type
/// of the sort column it belongs to (`order` is `col,-col[:suffix]`; a leading `-` is descending) so a
/// forged value is a 400 up front. Columns that are not numbers or timestamps are text and always fit.
fn check_keys(order: &str, keys: &Keys) -> Result<()> {
    let cols = order.split(':').next().unwrap_or("").split(',');
    for key in keys.iter().flatten() {
        // Postgres text cannot hold NUL (22021), whatever column the key is for
        if key.contains('\0') {
            return Err(Error::Invalid("bad cursor".to_string()));
        }
    }
    for (col, key) in cols.zip(keys) {
        let Some(key) = key else { continue };
        let ok = match col.trim_start_matches('-') {
            "number" | "priority" | "requested" | "rn" | "seq" | "position" | "served"
            | "last_seq" | "chars" => key.parse::<i64>().is_ok(),
            "created_at" | "updated_at" | "last_seen" => timestamp_fits(key),
            _ => true,
        };
        if !ok {
            return Err(Error::Invalid("bad cursor".to_string()));
        }
    }
    Ok(())
}

/// A timestamp key chrono parses and Postgres accepts as `timestamptz`: an offset under 16 hours
/// (Postgres refuses more, 22009) and a year in 1..=9999 both as written and in UTC (22007).
fn timestamp_fits(key: &str) -> bool {
    use chrono::Datelike;
    let Ok(t) = chrono::DateTime::parse_from_str(key, "%Y-%m-%d %H:%M:%S%.f%#z") else {
        return false;
    };
    let year_ok = |y: i32| (1..=9999).contains(&y);
    t.offset().local_minus_utc().abs() < 16 * 3600
        && year_ok(t.year())
        && year_ok(t.with_timezone(&chrono::Utc).year())
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
        Some(c) => {
            let keys = decode_cursor(c, order, nkeys).map_err(bad_cursor)?;
            check_keys(order, &keys)?;
            Some(keys)
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn keys(v: &[&str]) -> Keys {
        v.iter().map(|k| Some(k.to_string())).collect()
    }

    #[test]
    fn integer_keys_must_be_integers() {
        for col in [
            "number", "priority", "seq", "position", "served", "last_seq", "chars",
        ] {
            assert!(check_keys(col, &keys(&["12"])).is_ok(), "{col}");
            assert!(
                check_keys(&format!("-{col}"), &keys(&["-3"])).is_ok(),
                "-{col}"
            );
            assert!(check_keys(col, &keys(&["x"])).is_err(), "{col}");
            assert!(check_keys(col, &keys(&["1'; drop"])).is_err(), "{col}");
        }
    }

    #[test]
    fn timestamp_keys_must_be_timestamps() {
        let ts = "2026-05-01 10:11:12.123456+00";
        for col in ["created_at", "updated_at", "last_seen"] {
            assert!(
                check_keys(&format!("-{col},name"), &keys(&[ts, "a"])).is_ok(),
                "{col}"
            );
            assert!(
                check_keys(&format!("-{col},name"), &keys(&["yesterday", "a"])).is_err(),
                "{col}"
            );
        }
    }

    #[test]
    fn timestamps_postgres_would_refuse_are_refused() {
        for bad in [
            "2026-01-01 00:00:00+16",
            "2026-01-01 00:00:00-17:30",
            "-0001-01-01 00:00:00+00",
            "0000-06-01 00:00:00+00",
            "+10000-01-01 00:00:00+00",
            "0001-01-01 00:00:00+05",
            "9999-12-31 23:59:59-05",
        ] {
            assert!(check_keys("-created_at", &keys(&[bad])).is_err(), "{bad}");
        }
        for good in [
            "0001-01-01 00:00:00+00",
            "9999-12-31 23:59:59.999999+00",
            "2026-01-01 00:00:00+15:59",
            "2026-01-01 00:00:00-08",
        ] {
            assert!(check_keys("-created_at", &keys(&[good])).is_ok(), "{good}");
        }
    }

    #[test]
    fn integers_are_range_checked_to_i64() {
        assert!(check_keys("seq", &keys(&["9223372036854775807"])).is_ok());
        assert!(check_keys("seq", &keys(&["9223372036854775808"])).is_err());
        assert!(check_keys("-chars", &keys(&["-9223372036854775809"])).is_err());
    }

    #[test]
    fn nul_is_refused_in_any_key() {
        assert!(check_keys("name", &keys(&["a\0b"])).is_err());
        assert!(check_keys("id", &keys(&["\0"])).is_err());
        assert!(
            check_keys(
                "-created_at,ref:live",
                &keys(&["2026-01-01 00:00:00+00", "x\0"])
            )
            .is_err()
        );
        assert!(
            check_keys(
                "-created_at,ref:live",
                &keys(&["2026-01-01 00:00:00+00", "x"])
            )
            .is_ok()
        );
    }

    #[test]
    fn text_keys_always_fit_and_the_suffix_is_ignored() {
        assert!(check_keys("name", &keys(&["anything at all"])).is_ok());
        assert!(check_keys("-chars,cook:x", &keys(&["7", "bob"])).is_ok());
        assert!(check_keys("-chars,cook:x", &keys(&["seven", "bob"])).is_err());
        assert!(check_keys("-last_seen,norm_url", &vec![None, Some("u".into())]).is_ok());
    }
}
