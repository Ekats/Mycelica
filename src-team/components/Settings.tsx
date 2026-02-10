import { useState, useEffect, useCallback } from "react";
import { X, Wifi, WifiOff } from "lucide-react";
import { useTeamStore } from "../stores/teamStore";

export default function Settings() {
  const { config, setShowSettings, saveSettings } = useTeamStore();

  const [serverUrl, setServerUrl] = useState(config?.server_url || "http://localhost:3741");
  const [author, setAuthor] = useState(config?.author || "");
  const [testing, setTesting] = useState(false);
  const [testResult, setTestResult] = useState<{ ok: boolean; message: string } | null>(null);

  useEffect(() => {
    if (config) {
      setServerUrl(config.server_url);
      setAuthor(config.author);
    }
  }, [config]);

  const handleTest = useCallback(async () => {
    setTesting(true);
    setTestResult(null);
    try {
      // Temporarily save to test, then use team_refresh as a connectivity test
      // We'll just do a simple fetch to /health
      const resp = await fetch(`${serverUrl}/health`);
      if (resp.ok) {
        const data = await resp.json();
        setTestResult({ ok: true, message: `Connected: ${data.nodes} nodes, ${data.edges} edges` });
      } else {
        setTestResult({ ok: false, message: `Server returned ${resp.status}` });
      }
    } catch (e) {
      setTestResult({ ok: false, message: String(e) });
    } finally {
      setTesting(false);
    }
  }, [serverUrl]);

  const handleSave = useCallback(() => {
    saveSettings({ server_url: serverUrl, author: author.trim() || "anonymous" });
  }, [serverUrl, author, saveSettings]);

  return (
    <div className="modal-overlay" onClick={(e) => e.target === e.currentTarget && setShowSettings(false)}>
      <div className="modal-content" style={{ maxWidth: 440 }}>
        <div className="flex items-center justify-between mb-4">
          <h2 className="text-lg font-semibold">Settings</h2>
          <button className="btn-secondary p-1" onClick={() => setShowSettings(false)}>
            <X size={16} />
          </button>
        </div>

        {/* Server URL */}
        <div className="mb-4">
          <label className="block text-xs mb-1 font-medium" style={{ color: "var(--text-secondary)" }}>
            Server URL
          </label>
          <div className="flex gap-2">
            <input
              type="text"
              className="flex-1"
              value={serverUrl}
              onChange={(e) => setServerUrl(e.target.value)}
              placeholder="http://localhost:3741"
            />
            <button
              className="btn-secondary flex items-center gap-1"
              onClick={handleTest}
              disabled={testing}
            >
              {testing ? "..." : "Test"}
            </button>
          </div>
          {testResult && (
            <div className="flex items-center gap-1.5 mt-1.5 text-xs">
              {testResult.ok ? (
                <Wifi size={12} style={{ color: "#10b981" }} />
              ) : (
                <WifiOff size={12} style={{ color: "#ef4444" }} />
              )}
              <span style={{ color: testResult.ok ? "#10b981" : "#ef4444" }}>
                {testResult.message}
              </span>
            </div>
          )}
        </div>

        {/* Author */}
        <div className="mb-6">
          <label className="block text-xs mb-1 font-medium" style={{ color: "var(--text-secondary)" }}>
            Author Name
          </label>
          <input
            type="text"
            className="w-full"
            value={author}
            onChange={(e) => setAuthor(e.target.value)}
            placeholder="Your name"
          />
          <p className="text-[11px] mt-1" style={{ color: "var(--text-secondary)" }}>
            Attached to nodes and edges you create.
          </p>
        </div>

        {/* Actions */}
        <div className="flex justify-end gap-2">
          <button className="btn-secondary" onClick={() => setShowSettings(false)}>Cancel</button>
          <button className="btn-primary" onClick={handleSave}>Save</button>
        </div>
      </div>
    </div>
  );
}
