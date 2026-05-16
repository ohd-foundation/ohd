import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useState,
  type ReactNode,
} from "react";
import { api, ApiError, type Chat, type Connection } from "./api";

// Shared store for the user's Connections and their conversations. Both are
// fetched once and grouped client-side (a chat carries `source_id`). The
// sidebar and the Connection pages all read from here so a single refresh
// keeps every view consistent.
interface DataState {
  connections: Connection[] | null;
  chats: Chat[] | null;
  error: string | null;
  reload: () => Promise<void>;
  // Conversations belonging to one Connection, newest first.
  chatsFor: (connId: string) => Chat[];
  connectionById: (connId: string) => Connection | undefined;
}

const DataContext = createContext<DataState | null>(null);

export function DataProvider({ children }: { children: ReactNode }) {
  const [connections, setConnections] = useState<Connection[] | null>(null);
  const [chats, setChats] = useState<Chat[] | null>(null);
  const [error, setError] = useState<string | null>(null);

  const reload = useCallback(async () => {
    try {
      const [c, ch] = await Promise.all([api.connections(), api.chats()]);
      const conns = [...c.sources].sort((a, b) =>
        a.label.localeCompare(b.label),
      );
      const sorted = [...ch.chats].sort((a, b) =>
        b.updated_at.localeCompare(a.updated_at),
      );
      setConnections(conns);
      setChats(sorted);
      setError(null);
    } catch (e) {
      setError(
        e instanceof ApiError ? e.message : "Failed to load connections",
      );
    }
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  const value = useMemo<DataState>(() => {
    return {
      connections,
      chats,
      error,
      reload,
      chatsFor: (connId: string) =>
        (chats ?? []).filter((c) => c.source_id === connId),
      connectionById: (connId: string) =>
        (connections ?? []).find((c) => c.id === connId),
    };
  }, [connections, chats, error, reload]);

  return <DataContext.Provider value={value}>{children}</DataContext.Provider>;
}

export function useData(): DataState {
  const ctx = useContext(DataContext);
  if (!ctx) throw new Error("useData must be used within DataProvider");
  return ctx;
}
