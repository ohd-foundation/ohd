import { createContext, useCallback, useContext, useState, type ReactNode } from "react";

interface ToastEntry {
  id: number;
  message: string;
  variant: "default" | "success" | "error";
}

interface ToastContextShape {
  show: (message: string, variant?: ToastEntry["variant"]) => void;
}

const ToastContext = createContext<ToastContextShape | null>(null);

let nextId = 1;

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastEntry[]>([]);

  const show = useCallback((message: string, variant: ToastEntry["variant"] = "default") => {
    const id = nextId++;
    setItems((prev) => [...prev, { id, message, variant }]);
    setTimeout(() => {
      setItems((prev) => prev.filter((i) => i.id !== id));
    }, 4500);
  }, []);

  return (
    <ToastContext.Provider value={{ show }}>
      {children}
      <ToastViewport items={items} />
    </ToastContext.Provider>
  );
}

function ToastViewport({ items }: { items: ToastEntry[] }) {
  if (items.length === 0) return null;
  return (
    <div className="toast-wrap" role="status" aria-live="polite">
      {items.map((i) => (
        <div key={i.id} className={`toast ${i.variant !== "default" ? i.variant : ""}`.trim()}>
          {i.message}
        </div>
      ))}
    </div>
  );
}

export function useToast(): ToastContextShape {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error("useToast must be used inside <ToastProvider>");
  return ctx;
}

export function _resetToastIds() {
  nextId = 1;
}
