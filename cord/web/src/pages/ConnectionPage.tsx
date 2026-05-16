import { useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";
import { api, ApiError } from "../api";
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
