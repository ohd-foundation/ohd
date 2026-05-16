import { Link } from "react-router-dom";
import { Spinner } from "../components/common";
import { useData } from "../data";

// Landing pane. CORD has no flat conversation list — work always starts from
// a Connection, so Home points the user at one (or at adding their first).
export default function HomePage() {
  const { connections } = useData();

  if (!connections) return <Spinner />;

  return (
    <div className="page">
      <div className="page-head">
        <div>
          <h1>OHD CORD</h1>
          <p>Talk to your health data in natural language.</p>
        </div>
      </div>

      {connections.length === 0 ? (
        <div className="empty">
          <p style={{ marginTop: 0 }}>
            You have no connections yet. A connection links CORD to a
            health-data store it may read on your behalf.
          </p>
          <Link to="/connections/new">
            <button type="button" className="primary">
              Add a connection
            </button>
          </Link>
        </div>
      ) : (
        <>
          <h2 style={{ marginBottom: 10 }}>Your connections</h2>
          <p className="muted" style={{ marginTop: 0 }}>
            Select a connection to see its conversations and start a new one.
          </p>
          <div className="stack" style={{ marginTop: 14 }}>
            {connections.map((conn) => (
              <Link
                key={conn.id}
                to={`/connections/${conn.id}`}
                className="list-item"
                style={{ display: "block", color: "var(--text)" }}
              >
                <div className="spread">
                  <strong>{conn.label}</strong>
                  <span className="faint" style={{ fontSize: 12.5 }}>
                    {conn.status}
                  </span>
                </div>
              </Link>
            ))}
          </div>
        </>
      )}
    </div>
  );
}
