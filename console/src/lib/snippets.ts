export type CookKind = "claude-code" | "codex" | "opencode" | "copilot" | "antigravity" | "other";

export const KIND_LABEL: Record<string, string> = {
  "claude-code": "Claude Code",
  codex: "Codex",
  opencode: "OpenCode",
  copilot: "GitHub Copilot",
  antigravity: "Antigravity",
  other: "Other MCP client",
};

export const RUNTIME_LABEL: Record<string, string> = { local: "local", cloud: "cloud" };

export interface Snippet {
  id: string;
  title: string;
  file: string;
  body: string;
}

/**
 * The same MCP configs `corntchap-cli cook setup` prints (crates/cli/src/setup.rs), so a snippet copied from the
 * console and one printed by the CLI are interchangeable.
 */
export function mcpSnippets(server: string, token?: string | null): Snippet[] {
  const url = `${server.replace(/\/+$/, "")}/mcp`;
  const t = token ?? "<token>";
  return [
    {
      id: "claude-code",
      title: "Claude Code",
      file: ".mcp.json",
      body: `{
  "mcpServers": {
    "corntchap": {
      "type": "http",
      "url": "${url}",
      "headers": { "Authorization": "Bearer ${t}" }
    }
  }
}

# or: claude mcp add --transport http corntchap ${url} --header "Authorization: Bearer ${t}"`,
    },
    {
      id: "codex",
      title: "Codex",
      file: "~/.codex/config.toml",
      body: `[mcp_servers.corntchap]
url = "${url}"
http_headers = { Authorization = "Bearer ${t}" }`,
    },
    {
      id: "opencode",
      title: "OpenCode",
      file: "opencode.json",
      body: `{
  "$schema": "https://opencode.ai/config.json",
  "mcp": {
    "corntchap": {
      "type": "remote",
      "url": "${url}",
      "headers": { "Authorization": "Bearer ${t}" }
    }
  }
}`,
    },
    {
      id: "copilot",
      title: "Copilot (VS Code)",
      file: ".vscode/mcp.json",
      body: `{
  "servers": {
    "corntchap": {
      "type": "http",
      "url": "${url}",
      "headers": { "Authorization": "Bearer ${t}" }
    }
  }
}`,
    },
    {
      id: "copilot-cli",
      title: "Copilot CLI",
      file: "~/.copilot/mcp-config.json",
      body: `{
  "mcpServers": {
    "corntchap": {
      "type": "http",
      "url": "${url}",
      "headers": { "Authorization": "Bearer ${t}" },
      "tools": ["*"]
    }
  }
}`,
    },
    {
      id: "antigravity",
      title: "Antigravity",
      file: "~/.gemini/config/mcp_config.json",
      body: `{
  "mcpServers": {
    "corntchap": {
      "serverUrl": "${url}",
      "headers": { "Authorization": "Bearer ${t}" }
    }
  }
}`,
    },
  ];
}

export async function copyText(text: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(text);
    return true;
  } catch {
    try {
      const ta = document.createElement("textarea");
      ta.value = text;
      ta.style.position = "fixed";
      ta.style.opacity = "0";
      document.body.appendChild(ta);
      ta.select();
      const ok = document.execCommand("copy");
      ta.remove();
      return ok;
    } catch {
      return false;
    }
  }
}
