import { useEffect, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { api, ApiError, type ConnectionSummary } from "../api";
import {
  Empty,
  ErrorBanner,
  formatDate,
  Spinner,
  StatusPill,
} from "../components/common";
import { useData } from "../data";

// One Connection: its health/scope, its conversations, and the action to
// start a new conversation scoped to it. A conversation is always opened in
// the context of the Connection it belongs to.
export default function ConnectionPage() {
  const { connId } = useParams();
  const navigate = useNavigate();
  const { connections, error, reload, chatsFor, connectionById } = useData();

  const [busy, setBusy] = useState(false);
  const [actionError, setActionError] = useState<string | null>(null);

  const [summary, setSummary] = useState<ConnectionSummary | null>(null);
  const [summaryLoading, setSummaryLoading] = useState(true);

  // Fetch the data summary once the connection id is known. A failure here
  // is treated like an unreachable connection — never a hard error.
  useEffect(() => {
    if (!connId) return;
    let alive = true;
    setSummaryLoading(true);
    api
      .connectionSummary(connId)
      .then((s) => {
        if (alive) setSummary(s);
      })
      .catch(() => {
        if (alive) setSummary({ summary: null, status: "unreachable" });
      })
      .finally(() => {
        if (alive) setSummaryLoading(false);
      });
    return () => {
      alive = false;
    };
  }, [connId]);

  if (error) {
    return (
      <div className="page">
        <ErrorBanner message={error} />
      </div>
    );
  }
  if (!connections || !connId) return <Spinner />;

  const conn = connectionById(connId);
  if (!conn) {
    return (
      <div className="page">
        <div className="page-head">
          <div>
            <h1>Connection not found</h1>
            <p>This connection no longer exists.</p>
          </div>
        </div>
        <Link to="/">Back to home</Link>
      </div>
    );
  }

  const chats = chatsFor(connId);

  const onRefresh = async () => {
    setBusy(true);
    setActionError(null);
    try {
      await api.refreshConnection(connId);
      await reload();
    } catch (e) {
      setActionError(e instanceof ApiError ? e.message : "Refresh failed");
    } finally {
      setBusy(false);
    }
  };

  const onDisconnect = async () => {
    if (!confirm("Disconnect this connection? The stored credential is wiped."))
      return;
    setBusy(true);
    setActionError(null);
    try {
      await api.deleteConnection(connId);
      await reload();
      navigate("/");
    } catch (e) {
      setActionError(e instanceof ApiError ? e.message : "Disconnect failed");
      setBusy(false);
    }
  };

  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1>{conn.label}</h1>
          <p>
            <StatusPill status={conn.status} /> {conn.kind} · {conn.endpoint}
          </p>
        </div>
        <div className="row">
          <button className="small" disabled={busy} onClick={onRefresh}>
            Refresh
          </button>
          <button
            className="small danger"
            disabled={busy}
            onClick={onDisconnect}
          >
            Disconnect
          </button>
        </div>
      </div>

      {actionError && <ErrorBanner message={actionError} />}

      <div className="muted" style={{ fontSize: 12.5, marginBottom: 18 }}>
        <div>
          Connected {formatDate(conn.created_at)} · Last reachable{" "}
          {formatDate(conn.last_ok_at)}
        </div>
        {conn.scope_json && (
          <div className="faint" style={{ marginTop: 4 }}>
            Scope: {conn.scope_json}
          </div>
        )}
      </div>

      <DataSummaryPanel loading={summaryLoading} summary={summary} />

      <div className="spread" style={{ marginBottom: 12 }}>
        <h2>Conversations</h2>
        <Link to={`/connections/${connId}/new-conversation`}>
          <button type="button" className="primary small">
            New conversation
          </button>
        </Link>
      </div>

      {chats.length === 0 ? (
        <Empty>
          No conversations with this connection yet. Start one to ask about
          its health data.
        </Empty>
      ) : (
        <div className="stack">
          {chats.map((c) => (
            <Link
              key={c.id}
              to={`/chats/${c.id}`}
              className="list-item"
              style={{ display: "block", color: "var(--text)" }}
            >
              <div className="spread">
                <strong>{c.title || "New conversation"}</strong>
                <span className="faint" style={{ fontSize: 12.5 }}>
                  {c.model || "default model"}
                </span>
              </div>
              <div className="faint" style={{ marginTop: 4, fontSize: 12.5 }}>
                Updated {formatDate(c.updated_at)}
              </div>
            </Link>
          ))}
        </div>
      )}
    </div>
  );
}

// A compact, read-only view of what data the connection holds: total event
// count, the most recent event date, and the event types present. Sourced
// from the connection's `describe_data` tool via GET /v1/sources/:id/summary.
function DataSummaryPanel({
  loading,
  summary,
}: {
  loading: boolean;
  summary: ConnectionSummary | null;
}) {
  if (loading) {
    return (
      <div className="card" style={{ marginBottom: 18 }}>
        <div className="faint" style={{ fontSize: 12.5 }}>
          Loading data summary…
        </div>
      </div>
    );
  }

  // Null summary: an offline phone-backed connection is normal, not an error.
  if (!summary || !summary.summary) {
    return (
      <div className="card" style={{ marginBottom: 18 }}>
        <h3 style={{ margin: "0 0 4px" }}>Data summary</h3>
        <div className="faint" style={{ fontSize: 12.5 }}>
          The connection is currently unreachable, so its data summary is
          unavailable. This is expected when a phone-backed storage is offline.
        </div>
      </div>
    );
  }

  const data = summary.summary;
  const types = data.event_types ?? [];
  const latest = types
    .map((t) => t.latest_iso)
    .filter((iso): iso is string => !!iso)
    .sort();
  const newest = latest.length ? latest[latest.length - 1] : null;

  return (
    <div className="card" style={{ marginBottom: 18 }}>
      <h3 style={{ margin: "0 0 8px" }}>Data summary</h3>
      <div
        className="row"
        style={{ gap: 24, flexWrap: "wrap", marginBottom: types.length ? 10 : 0 }}
      >
        <div>
          <div style={{ fontSize: 20, fontWeight: 600, color: "var(--accent)" }}>
            {data.total_events.toLocaleString()}
          </div>
          <div className="faint" style={{ fontSize: 12 }}>
            total events
          </div>
        </div>
        <div>
          <div style={{ fontSize: 20, fontWeight: 600, color: "var(--accent)" }}>
            {types.length}
          </div>
          <div className="faint" style={{ fontSize: 12 }}>
            event types
          </div>
        </div>
        <div>
          <div style={{ fontSize: 14, fontWeight: 600 }}>
            {newest ? formatDate(newest) : "—"}
          </div>
          <div className="faint" style={{ fontSize: 12 }}>
            most recent event
          </div>
        </div>
      </div>
      {types.length > 0 && (
        <div className="faint" style={{ fontSize: 12.5 }}>
          {types
            .map((t) => `${t.event_type} (${t.count.toLocaleString()})`)
            .join(" · ")}
        </div>
      )}
    </div>
  );
}
