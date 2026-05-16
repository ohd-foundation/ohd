import { NavLink, Outlet, useNavigate, useParams } from "react-router-dom";
import { api, ApiError, type Chat, type Connection } from "../api";
import { useAuth } from "../auth";
import { DataProvider, useData } from "../data";

// The app shell: left sidebar (Connections + their conversations) and the
// routed main pane. Conversations are grouped under the Connection they
// belong to rather than shown as one flat global list.
export default function Layout() {
  return (
    <DataProvider>
      <Shell />
    </DataProvider>
  );
}

function Shell() {
  const { me, logout } = useAuth();
  const navigate = useNavigate();
  const params = useParams();
  const activeChatId = params.chatId;
  const activeConnId = params.connId;

  const { connections, chats, error, reload, chatsFor } = useData();

  // Which Connection owns the conversation in the route, so its group can be
  // shown expanded/active even when the URL is /chats/:id.
  const activeChat = (chats ?? []).find((c) => c.id === activeChatId);
  const selectedConnId = activeConnId ?? activeChat?.source_id ?? null;

  const onLogout = async () => {
    await logout();
    navigate("/login");
  };

  return (
    <div className="app-shell">
      <aside className="sidebar">
        <div className="sidebar-head">
          <div className="brand">OHD CORD</div>
          <div className="brand-sub">conversational health data</div>
        </div>

        <nav className="nav">
          <NavLink to="/" end>
            Home
          </NavLink>
          <NavLink to="/models">Models</NavLink>
        </nav>

        <div className="conn-list">
          <div className="conn-list-head">
            <h3>Connections</h3>
          </div>

          {error && <ErrorLine text={error} />}

          {!error && connections && connections.length === 0 && (
            <p className="conn-empty">No connections yet.</p>
          )}

          {!error &&
            connections &&
            connections.map((conn) => (
              <ConnectionGroup
                key={conn.id}
                conn={conn}
                chats={chatsFor(conn.id)}
                selected={selectedConnId === conn.id}
                activeChatId={activeChatId}
                onChatsChanged={reload}
              />
            ))}

          <button
            type="button"
            className="primary small add-conn-btn"
            onClick={() => navigate("/connections/new")}
          >
            Add a connection
          </button>
        </div>

        <div className="sidebar-foot">
          <span title={me?.user.cord_user_ulid}>
            {me?.user.display_label || "Signed in"}
          </span>
          <button className="ghost small" onClick={onLogout}>
            Sign out
          </button>
        </div>
      </aside>

      <main className="main">
        <Outlet />
      </main>
    </div>
  );
}

// One Connection in the sidebar plus its nested conversations.
function ConnectionGroup({
  conn,
  chats,
  selected,
  activeChatId,
  onChatsChanged,
}: {
  conn: Connection;
  chats: Chat[];
  selected: boolean;
  activeChatId: string | undefined;
  onChatsChanged: () => void | Promise<void>;
}) {
  const navigate = useNavigate();

  const onDeleteChat = async (id: string) => {
    if (!confirm("Delete this conversation?")) return;
    try {
      await api.deleteChat(id);
      if (activeChatId === id) navigate(`/connections/${conn.id}`);
      await onChatsChanged();
    } catch (e) {
      alert(e instanceof ApiError ? e.message : "Failed to delete chat");
    }
  };

  return (
    <div className="conn-group">
      <div className={"conn-row" + (selected ? " active" : "")}>
        <NavLink to={`/connections/${conn.id}`} title={conn.label}>
          <span
            className={`conn-status-dot ${statusDot(conn.status)}`}
            style={{ display: "inline-block", marginRight: 7 }}
          />
          {conn.label}
        </NavLink>
      </div>

      {selected && (
        <div className="conn-children">
          {chats.length === 0 && (
            <div className="conn-empty">No conversations yet.</div>
          )}
          {chats.map((c) => (
            <div
              key={c.id}
              className={
                "chat-row" + (activeChatId === c.id ? " active" : "")
              }
            >
              <NavLink to={`/chats/${c.id}`} title={chatLabel(c)}>
                {chatLabel(c)}
              </NavLink>
              <button
                className="ghost small del"
                title="Delete"
                onClick={() => onDeleteChat(c.id)}
              >
                ✕
              </button>
            </div>
          ))}
          <button
            type="button"
            className="conn-new"
            onClick={() =>
              navigate(`/connections/${conn.id}/new-conversation`)
            }
          >
            + New conversation
          </button>
        </div>
      )}
    </div>
  );
}

function statusDot(status: string): string {
  const s = status.toLowerCase();
  if (s === "ok" || s === "reachable" || s === "connected") return "ok";
  if (s === "error" || s === "unreachable" || s === "failed") return "bad";
  if (s === "pending" || s === "stale" || s === "unknown") return "warn";
  return "";
}

// A chat's sidebar label: its auto-derived title, or — while the title is
// still null (no message sent yet) — the date it was created.
function chatLabel(c: Chat): string {
  if (c.title && c.title.trim() !== "") return c.title;
  const d = new Date(c.created_at);
  return Number.isNaN(d.getTime())
    ? "New conversation"
    : `New conversation · ${d.toLocaleDateString()}`;
}

function ErrorLine({ text }: { text: string }) {
  return (
    <p
      style={{ padding: "4px 10px", fontSize: 12.5 }}
      className="banner error"
    >
      {text}
    </p>
  );
}
