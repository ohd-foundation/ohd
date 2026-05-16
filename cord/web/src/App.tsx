import { Navigate, Route, Routes } from "react-router-dom";
import { AuthProvider, useAuth } from "./auth";
import { Spinner } from "./components/common";
import Layout from "./components/Layout";
import LoginPage from "./pages/LoginPage";
import HomePage from "./pages/HomePage";
import ConnectionPage from "./pages/ConnectionPage";
import NewChatPage from "./pages/NewChatPage";
import ChatPage from "./pages/ChatPage";
import ConnectionsPage from "./pages/ConnectionsPage";
import ModelsPage from "./pages/ModelsPage";

// Gates the app shell on an authenticated session.
function RequireAuth() {
  const { me, loading } = useAuth();
  if (loading) return <Spinner />;
  if (!me) return <Navigate to="/login" replace />;
  return <Layout />;
}

function LoginGate() {
  const { me, loading } = useAuth();
  if (loading) return <Spinner />;
  if (me) return <Navigate to="/" replace />;
  return <LoginPage />;
}

export default function App() {
  return (
    <AuthProvider>
      <Routes>
        <Route path="/login" element={<LoginGate />} />
        <Route element={<RequireAuth />}>
          <Route path="/" element={<HomePage />} />
          <Route path="/connections/new" element={<ConnectionsPage />} />
          <Route path="/connections/:connId" element={<ConnectionPage />} />
          <Route
            path="/connections/:connId/new-conversation"
            element={<NewChatPage />}
          />
          <Route path="/chats/:chatId" element={<ChatPage />} />
          <Route path="/models" element={<ModelsPage />} />
        </Route>
        <Route path="*" element={<Navigate to="/" replace />} />
      </Routes>
    </AuthProvider>
  );
}
