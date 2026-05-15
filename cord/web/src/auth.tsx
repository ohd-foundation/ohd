import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useState,
  type ReactNode,
} from "react";
import { api, ApiError, type Me } from "./api";

interface AuthState {
  me: Me | null;
  loading: boolean;
  // null = unknown / not yet loaded
  reload: () => Promise<void>;
  logout: () => Promise<void>;
}

const AuthContext = createContext<AuthState | null>(null);

export function AuthProvider({ children }: { children: ReactNode }) {
  const [me, setMe] = useState<Me | null>(null);
  const [loading, setLoading] = useState(true);

  const reload = useCallback(async () => {
    setLoading(true);
    try {
      const data = await api.me();
      setMe(data);
    } catch (e) {
      // 401 (and anything else) leaves the user unauthenticated.
      if (!(e instanceof ApiError)) console.error(e);
      setMe(null);
    } finally {
      setLoading(false);
    }
  }, []);

  const logout = useCallback(async () => {
    try {
      await api.logout();
    } catch {
      // ignore — clear local state regardless
    }
    setMe(null);
  }, []);

  useEffect(() => {
    void reload();
  }, [reload]);

  return (
    <AuthContext.Provider value={{ me, loading, reload, logout }}>
      {children}
    </AuthContext.Provider>
  );
}

export function useAuth(): AuthState {
  const ctx = useContext(AuthContext);
  if (!ctx) throw new Error("useAuth must be used within AuthProvider");
  return ctx;
}
