import { useCallback, useEffect, useRef, useState } from "react";
import { Link, useParams } from "react-router-dom";
import {
  api,
  ApiError,
  streamMessage,
  type ChatDetail,
  type ChatMessage,
} from "../api";
import { ErrorBanner, InfoBanner, Spinner } from "../components/common";
import { useData } from "../data";

// A message that may still be streaming (no persisted id yet).
interface LocalMessage extends Omit<ChatMessage, "id"> {
  id: string;
  pending?: boolean;
}

export default function ChatPage() {
  const { chatId } = useParams();
  const { connectionById } = useData();
  const [detail, setDetail] = useState<ChatDetail | null>(null);
  const [loadError, setLoadError] = useState<string | null>(null);

  const [messages, setMessages] = useState<LocalMessage[]>([]);
  const [draft, setDraft] = useState("");
  const [sending, setSending] = useState(false);
  const [toolStatus, setToolStatus] = useState<string | null>(null);
  const [sendError, setSendError] = useState<string | null>(null);
  const [agentUnavailable, setAgentUnavailable] = useState(false);

  const scrollRef = useRef<HTMLDivElement>(null);

  // Load history whenever the route's chat id changes.
  useEffect(() => {
    if (!chatId) return;
    setDetail(null);
    setLoadError(null);
    setMessages([]);
    setAgentUnavailable(false);
    setSendError(null);
    api
      .chat(chatId)
      .then((d) => {
        setDetail(d);
        setMessages(d.messages.map((m) => ({ ...m })));
      })
      .catch((e) =>
        setLoadError(
          e instanceof ApiError ? e.message : "Failed to load conversation",
        ),
      );
  }, [chatId]);

  const scrollToBottom = useCallback(() => {
    const el = scrollRef.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, []);

  useEffect(() => {
    scrollToBottom();
  }, [messages, toolStatus, scrollToBottom]);

  const send = async () => {
    const text = draft.trim();
    if (!text || !chatId || sending) return;

    const now = new Date().toISOString();
    const userMsg: LocalMessage = {
      id: `local-user-${Date.now()}`,
      role: "user",
      content: text,
      created_at: now,
    };
    const assistantId = `local-asst-${Date.now()}`;
    const assistantMsg: LocalMessage = {
      id: assistantId,
      role: "assistant",
      content: "",
      created_at: now,
      pending: true,
    };

    setMessages((ms) => [...ms, userMsg, assistantMsg]);
    setDraft("");
    setSending(true);
    setSendError(null);
    setToolStatus(null);

    const appendDelta = (delta: string) => {
      setMessages((ms) =>
        ms.map((m) =>
          m.id === assistantId ? { ...m, content: m.content + delta } : m,
        ),
      );
    };
    const finalize = () => {
      setMessages((ms) =>
        ms.map((m) =>
          m.id === assistantId ? { ...m, pending: false } : m,
        ),
      );
    };

    try {
      await streamMessage(chatId, text, (ev) => {
        switch (ev.type) {
          case "text":
            appendDelta(ev.delta);
            break;
          case "tool":
            setToolStatus(`calling ${ev.name}`);
            break;
          case "done":
            finalize();
            setToolStatus(null);
            break;
          case "error":
            setSendError(ev.message);
            finalize();
            setToolStatus(null);
            break;
        }
      });
      finalize();
      setToolStatus(null);
    } catch (e) {
      // Drop the empty pending assistant bubble on failure.
      setMessages((ms) => ms.filter((m) => m.id !== assistantId));
      setToolStatus(null);
      if (e instanceof ApiError && e.status === 501) {
        setAgentUnavailable(true);
      } else {
        setSendError(
          e instanceof ApiError ? e.message : "Failed to send message",
        );
      }
    } finally {
      setSending(false);
    }
  };

  const onKeyDown = (e: React.KeyboardEvent<HTMLTextAreaElement>) => {
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void send();
    }
  };

  if (loadError) {
    return (
      <div className="page">
        <ErrorBanner message={loadError} />
      </div>
    );
  }
  if (!detail) return <Spinner />;

  const conn = connectionById(detail.chat.source_id);

  return (
    <div className="chat-view">
      <div className="chat-header">
        <span>
          <span className="title">
            {detail.chat.title || "Untitled conversation"}
          </span>
          <span className="faint" style={{ fontSize: 12.5, marginLeft: 8 }}>
            ·{" "}
            <Link to={`/connections/${detail.chat.source_id}`}>
              {conn?.label || "connection"}
            </Link>
          </span>
        </span>
        <span className="faint" style={{ fontSize: 12.5 }}>
          {detail.chat.model || "default model"}
        </span>
      </div>

      <div className="chat-scroll" ref={scrollRef}>
        <div className="messages">
          {messages.length === 0 && (
            <p className="faint" style={{ textAlign: "center" }}>
              Send a message to begin. The agent reads this connection on
              your behalf.
            </p>
          )}
          {messages.map((m) => (
            <div key={m.id} className={`msg ${m.role}`}>
              <div className="bubble">
                {m.content ||
                  (m.pending ? <span className="faint">…</span> : "")}
              </div>
            </div>
          ))}
        </div>

        {toolStatus && <div className="tool-status">{toolStatus}…</div>}

        {agentUnavailable && (
          <div
            className="messages"
            style={{ marginTop: 16 }}
          >
            <InfoBanner>
              The chat agent is not yet available on this deployment. The
              conversation interface is ready; responses will work once the
              agent ships.
            </InfoBanner>
          </div>
        )}

        {sendError && (
          <div className="messages" style={{ marginTop: 16 }}>
            <ErrorBanner message={sendError} />
          </div>
        )}
      </div>

      <div className="composer">
        <div className="composer-inner">
          <textarea
            rows={1}
            placeholder="Ask about your health data…"
            value={draft}
            onChange={(e) => setDraft(e.target.value)}
            onKeyDown={onKeyDown}
            disabled={sending}
          />
          <button
            className="primary"
            onClick={() => void send()}
            disabled={sending || draft.trim() === ""}
          >
            {sending ? "…" : "Send"}
          </button>
        </div>
      </div>
    </div>
  );
}
