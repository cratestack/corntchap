//! Config snippets printed by `cook setup` / `agent enroll`.

/// The MCP client configuration for an agent of `kind`, with its token filled in.
pub fn mcp_snippet(kind: &str, server: &str, token: &str) -> String {
    let url = format!("{}/mcp", server.trim_end_matches('/'));
    match kind {
        "claude-code" => format!(
            "Claude Code, .mcp.json in the project (do not commit the token):\n\n{{\n  \"mcpServers\": {{\n    \"corntchap\": {{\n      \"type\": \"http\",\n      \"url\": \"{url}\",\n      \"headers\": {{ \"Authorization\": \"Bearer {token}\" }}\n    }}\n  }}\n}}\n\nor: claude mcp add --transport http corntchap {url} --header \"Authorization: Bearer {token}\"\n"
        ),
        "codex" => format!(
            "Codex, ~/.codex/config.toml:\n\n[mcp_servers.corntchap]\nurl = \"{url}\"\nhttp_headers = {{ Authorization = \"Bearer {token}\" }}\n"
        ),
        "opencode" => format!(
            "OpenCode, opencode.json:\n\n{{\n  \"$schema\": \"https://opencode.ai/config.json\",\n  \"mcp\": {{\n    \"corntchap\": {{\n      \"type\": \"remote\",\n      \"url\": \"{url}\",\n      \"headers\": {{ \"Authorization\": \"Bearer {token}\" }}\n    }}\n  }}\n}}\n"
        ),
        _ => format!(
            "Any MCP client (streamable HTTP):\n\n  url:     {url}\n  header:  Authorization: Bearer {token}\n"
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snippets_carry_url_and_token() {
        for k in ["claude-code", "codex", "opencode", "other"] {
            let s = mcp_snippet(k, "https://x.example/", "ct_abc");
            assert!(
                s.contains("https://x.example/mcp") && s.contains("Bearer ct_abc"),
                "{k}"
            );
        }
        assert!(mcp_snippet("codex", "http://h", "t").contains("[mcp_servers.corntchap]"));
        assert!(mcp_snippet("opencode", "http://h", "t").contains("\"type\": \"remote\""));
        // the JSON ones are valid JSON
        let s = mcp_snippet("opencode", "http://h", "t");
        let json = &s[s.find('{').unwrap()..];
        serde_json::from_str::<serde_json::Value>(json).unwrap();
    }
}
