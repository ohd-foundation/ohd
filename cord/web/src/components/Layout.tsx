import { useCallback, useEffect, useState } from "react";
import { NavLink, useNavigate, useParams } from "react-router-dom";
import { Outlet } from "react-router-dom";
import { api, ApiError, type Chat } from "../api";
import { useAuth } from "../auth";

// The app shell: left sidebar (nav + chat list) and the routed main pane.
export default function Layout() {
  const { me, logout } = useAuth();
  const navigate = useNavigate();
  const params = useParams();
  const activeChatId = params.chatId;

  const [chats, setChats] = useState<Chat[]>([]);
  const [chatsError, setChatsError] = useState<string | null>(null);

  const loadChats = useCallback(async () => {
    try {
      const data = await api.chats();
      data.chats.sort((a, b) => b.updated_at.localeCompare(a.updated_at));
      setChats(data.chats);
      setChatsError(null);
    } catch (e) {
      setChatsError(e instanceof ApiError ? e.message : "Failed to load chats");
    }
  }, []);

  useEffect(() => {
    void loadChats();
  }, [loadChats, activeChatId]);

  const onDeleteChat = async (id: string) => {
    if (!confirm("Delete this conversation?")) return;
    try {
      await api.deleteChat(id);
      setChats((cs) => cs.filter((c) => c.id !== id));
      if (activeChatId === id) navigate("/");
    } catch (e) {
      alert(e instanceof ApiError ? e.message : "Failed to delete chat");
    }
  };

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
            New chat
          </NavLink>
          <NavLink to="/sources">Data sources</NavLink>
          <NavLink to="/models">Models</NavLink>
        </nav>

        <div className="chat-list">
          <div className="chat-list-head">
            <h3>Conversations</h3>
          </div>
          {chatsError && <ErrorLine text={chatsError} />}
          {!chatsError && chats.length === 0 && (
            <p className="faint" style={{ padding: "4px 10px", fontSize: 12.5 }}>
              No conversations yet.
            </p>
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
