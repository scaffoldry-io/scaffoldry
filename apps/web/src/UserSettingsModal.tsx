import React, { useState, useEffect } from "react";
import { apiClient } from "./api";

export interface UserSettingsModalProps {
  isOpen: boolean;
  onClose: () => void;
  eppn: string;
}

export interface ApiTokenItem {
  id: string;
  label: string;
  kind: string;
  created_at: string;
  expires_at: string;
  last_used_at: string | null;
}

export const UserSettingsModal: React.FC<UserSettingsModalProps> = ({ isOpen, onClose, eppn }) => {
  const [tokens, setTokens] = useState<ApiTokenItem[]>([]);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [agentEnabled, setAgentEnabled] = useState(true);
  const [maxDays, setMaxDays] = useState(90);

  // Form state
  const [label, setLabel] = useState("");
  const [days, setDays] = useState(30);
  const [mintedSecret, setMintedSecret] = useState<string | null>(null);

  useEffect(() => {
    if (isOpen) {
      setMintedSecret(null);
      loadSettingsAndTokens();
    }
  }, [isOpen]);

  const loadSettingsAndTokens = async () => {
    setLoading(true);
    setError(null);
    try {
      const settingsRes = await apiClient.getSettings().catch(() => null);
      if (settingsRes?.settings) {
        if (settingsRes.settings["tokens.agent_enabled"] !== undefined) {
          setAgentEnabled(Boolean(settingsRes.settings["tokens.agent_enabled"]));
        }
        if (settingsRes.settings["tokens.max_days"] !== undefined) {
          const md = Number(settingsRes.settings["tokens.max_days"]);
          if (!isNaN(md) && md > 0) {
            setMaxDays(md);
            if (days > md) setDays(md);
          }
        }
      }

      const tokensRes = await apiClient.listTokens();
      if (tokensRes?.tokens) {
        setTokens(tokensRes.tokens);
      }
    } catch (e: any) {
      setError(e?.message || "Failed to load settings or tokens");
    } finally {
      setLoading(false);
    }
  };

  const handleCreateToken = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!label.trim()) return;
    setError(null);
    try {
      const res = await apiClient.createToken({
        label: label.trim(),
        days: Number(days),
        kind: "agent",
      });
      setMintedSecret(res.token);
      setLabel("");
      // reload token list
      const tokensRes = await apiClient.listTokens().catch(() => null);
      if (tokensRes?.tokens) {
        setTokens(tokensRes.tokens);
      }
    } catch (e: any) {
      setError(e?.message || "Failed to create token");
    }
  };

  const handleRevoke = async (id: string) => {
    setError(null);
    try {
      await apiClient.revokeToken(id);
      setTokens((prev) => prev.filter((t) => t.id !== id));
    } catch (e: any) {
      setError(e?.message || "Failed to revoke token");
    }
  };

  if (!isOpen) return null;

  const mcpUrl = typeof window !== "undefined"
    ? `${window.location.origin}/api/mcp`
    : "/api/mcp";

  return (
    <div
      data-testid="user-settings"
      className="fixed inset-0 z-50 flex items-center justify-center bg-black/50 backdrop-blur-xs p-4 animate-fade-in"
      role="dialog"
      aria-modal="true"
    >
      <div className="bg-white dark:bg-slate-900 border border-slate-200 dark:border-slate-800 rounded-xl shadow-2xl max-w-2xl w-full p-6 max-h-[90vh] overflow-y-auto space-y-6">
        <div className="flex items-center justify-between border-b border-slate-100 dark:border-slate-800 pb-3">
          <div>
            <h2 className="text-lg font-bold text-slate-900 dark:text-white">User Settings</h2>
            <p className="text-xs text-slate-500 dark:text-slate-400 font-mono">{eppn}</p>
          </div>
          <button
            type="button"
            onClick={onClose}
            className="text-slate-400 hover:text-slate-600 dark:hover:text-slate-200 text-lg font-bold p-1 cursor-pointer"
            aria-label="Close"
          >
            ✕
          </button>
        </div>

        {error && (
          <div className="p-3 bg-red-50 dark:bg-red-950/40 border border-red-200 dark:border-red-800 rounded-lg text-xs text-red-700 dark:text-red-300">
            {error}
          </div>
        )}

        {agentEnabled && (
          <div className="space-y-4">
            <div className="flex items-center justify-between">
              <h3 className="text-sm font-semibold text-slate-900 dark:text-white uppercase tracking-wider">
                Agent tokens
              </h3>
              <span className="text-xs text-slate-500">Max lifetime: {maxDays} days</span>
            </div>

            {/* Minted secret alert banner */}
            {mintedSecret && (
              <div className="p-4 bg-amber-50 dark:bg-amber-950/40 border border-amber-300 dark:border-amber-700 rounded-lg space-y-2">
                <div className="text-xs font-bold text-amber-800 dark:text-amber-200">
                  Copy this token now. It is not shown again.
                </div>
                <div className="flex items-center gap-2">
                  <input
                    data-testid="agent-token-secret"
                    type="text"
                    readOnly
                    value={mintedSecret}
                    className="flex-1 px-3 py-1.5 bg-white dark:bg-slate-900 border border-amber-300 dark:border-amber-700 rounded font-mono text-xs text-slate-900 dark:text-white select-all"
                  />
                  <button
                    type="button"
                    onClick={() => navigator.clipboard?.writeText(mintedSecret)}
                    className="px-3 py-1.5 bg-amber-600 hover:bg-amber-700 text-white rounded text-xs font-semibold cursor-pointer"
                  >
                    Copy
                  </button>
                </div>
                <div className="text-[11px] text-slate-600 dark:text-slate-400 flex items-center gap-1">
                  <span>MCP URL:</span>
                  <code className="font-mono bg-slate-100 dark:bg-slate-800 px-1 py-0.5 rounded">{mcpUrl}</code>
                </div>
              </div>
            )}

            {/* Token Creation Form */}
            <form data-testid="agent-token-form" onSubmit={handleCreateToken} className="flex gap-2 items-end bg-slate-50 dark:bg-slate-800/40 p-3 rounded-lg border border-slate-200 dark:border-slate-800">
              <div className="flex-1">
                <label className="block text-xs font-medium text-slate-700 dark:text-slate-300 mb-1">
                  Label
                </label>
                <input
                  type="text"
                  required
                  value={label}
                  onChange={(e) => setLabel(e.target.value)}
                  placeholder="e.g. Cursor Assistant, Research Script"
                  className="w-full px-3 py-1.5 border border-slate-300 dark:border-slate-700 rounded text-xs bg-white dark:bg-slate-900 text-slate-900 dark:text-white"
                />
              </div>
              <div className="w-24">
                <label className="block text-xs font-medium text-slate-700 dark:text-slate-300 mb-1">
                  Days
                </label>
                <input
                  type="number"
                  min="1"
                  max={maxDays}
                  value={days}
                  onChange={(e) => setDays(Math.min(maxDays, Math.max(1, parseInt(e.target.value) || 1)))}
                  className="w-full px-3 py-1.5 border border-slate-300 dark:border-slate-700 rounded text-xs bg-white dark:bg-slate-900 text-slate-900 dark:text-white"
                />
              </div>
              <button
                type="submit"
                className="px-4 py-1.5 bg-blue-600 hover:bg-blue-700 text-white rounded text-xs font-semibold cursor-pointer transition-colors"
              >
                Mint
              </button>
            </form>

            {/* Token List */}
            <div data-testid="agent-token-list" className="border border-slate-200 dark:border-slate-800 rounded-lg overflow-hidden">
              <table className="w-full text-left text-xs">
                <thead className="bg-slate-50 dark:bg-slate-800/60 text-slate-500 border-b border-slate-200 dark:border-slate-800">
                  <tr>
                    <th className="px-3 py-2 font-medium">Label</th>
                    <th className="px-3 py-2 font-medium">Created</th>
                    <th className="px-3 py-2 font-medium">Expires</th>
                    <th className="px-3 py-2 font-medium">Last Used</th>
                    <th className="px-3 py-2 font-medium text-right">Action</th>
                  </tr>
                </thead>
                <tbody className="divide-y divide-slate-100 dark:divide-slate-800">
                  {tokens.length === 0 ? (
                    <tr>
                      <td colSpan={5} className="px-3 py-4 text-center text-slate-400">
                        No agent tokens created yet.
                      </td>
                    </tr>
                  ) : (
                    tokens.map((tok) => (
                      <tr key={tok.id} className="hover:bg-slate-50 dark:hover:bg-slate-800/30">
                        <td className="px-3 py-2 font-medium text-slate-900 dark:text-white">{tok.label}</td>
                        <td className="px-3 py-2 text-slate-500 font-mono text-[11px]">{tok.created_at.split("T")[0]}</td>
                        <td className="px-3 py-2 text-slate-500 font-mono text-[11px]">{tok.expires_at.split("T")[0]}</td>
                        <td className="px-3 py-2 text-slate-500 font-mono text-[11px]">{tok.last_used_at ? tok.last_used_at.split("T")[0] : "Never"}</td>
                        <td className="px-3 py-2 text-right">
                          <button
                            type="button"
                            onClick={() => handleRevoke(tok.id)}
                            className="px-2 py-1 bg-red-50 hover:bg-red-100 text-red-600 dark:bg-red-950/40 dark:text-red-300 rounded font-medium text-[11px] cursor-pointer"
                          >
                            Revoke
                          </button>
                        </td>
                      </tr>
                    ))
                  )}
                </tbody>
              </table>
            </div>
          </div>
        )}

        <div className="flex justify-end pt-3 border-t border-slate-100 dark:border-slate-800">
          <button
            type="button"
            onClick={onClose}
            className="px-4 py-2 bg-slate-100 dark:bg-slate-800 hover:bg-slate-200 dark:hover:bg-slate-700 text-slate-700 dark:text-slate-300 rounded-lg text-xs font-semibold cursor-pointer"
          >
            Close
          </button>
        </div>
      </div>
    </div>
  );
};
