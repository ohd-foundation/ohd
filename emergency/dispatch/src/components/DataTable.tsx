import { useMemo, useState, type ReactNode } from "react";

export interface Column<T> {
  key: string;
  header: string;
  /** Render the cell. Defaults to `String(row[key])` if omitted. */
  cell?: (row: T) => ReactNode;
  /** Comparator for sorting. If omitted, the column is non-sortable. */
  sort?: (a: T, b: T) => number;
  /** "num" right-aligns + uses mono font. */
  align?: "left" | "right" | "num";
  /** CSS width hint (e.g. "120px", "30%"). */
  width?: string;
}

interface DataTableProps<T> {
  columns: Column<T>[];
  rows: T[];
  /** Click handler for entire rows (drawer open / row select). */
  onRowClick?: (row: T) => void;
  /** Optional filter input — applied with `filterMatch` if provided. */
  filterText?: string;
  filterMatch?: (row: T, q: string) => boolean;
  empty?: ReactNode;
  /** Extra className for individual rows (e.g. "row-selected"). */
  rowClassName?: (row: T) => string | undefined;
  /** Stable string id for the row (used as the React key). */
  rowKey: (row: T) => string;
}

interface SortState {
  key: string;
  dir: "asc" | "desc";
}

/**
 * Reusable dense table component.
 *
 * Information density is the dispatch console's whole point — this table
 * is intentionally compact (12px rows, monospace numerics, hover affordance).
 */
export function DataTable<T>({
  columns,
  rows,
  onRowClick,
  filterText,
  filterMatch,
  empty,
  rowClassName,
  rowKey,
}: DataTableProps<T>) {
  const [sort, setSort] = useState<SortState | null>(null);

  const visible = useMemo(() => {
    let out = rows;
    if (filterText && filterMatch) {
      const q = filterText.trim().toLowerCase();
      if (q) out = out.filter((r) => filterMatch(r, q));
    }
    if (sort) {
      const col = columns.find((c) => c.key === sort.key);
      if (col?.sort) {
        const cmp = col.sort;
        out = [...out].sort((a, b) => (sort.dir === "asc" ? cmp(a, b) : cmp(b, a)));
      }
    }
    return out;
  }, [rows, columns, sort, filterText, filterMatch]);

  function toggleSort(col: Column<T>) {
    if (!col.sort) return;
    setSort((s) => {
      if (!s || s.key !== col.key) return { key: col.key, dir: "asc" };
      if (s.dir === "asc") return { key: col.key, dir: "desc" };
      return null;
    });
  }

  return (
    <div className="data-table-wrap">
      <table className="data-table">
        <thead>
          <tr>
            {columns.map((c) => {
              const isSort = sort?.key === c.key;
              const sortable = !!c.sort;
              return (
                <th
                  key={c.key}
                  style={c.width ? { width: c.width } : undefined}
                  className={`${c.align === "num" ? "num" : ""} ${sortable ? "sortable" : ""}`}
                  onClick={sortable ? () => toggleSort(c) : undefined}
                >
                  <span>{c.header}</span>
                  {isSort && (
                    <span className="sort-arrow">{sort.dir === "asc" ? "▲" : "▼"}</span>
                  )}
                </th>
              );
            })}
          </tr>
        </thead>
        <tbody>
          {visible.length === 0 && (
            <tr>
              <td colSpan={columns.length} className="empty-row">
                {empty ?? "No rows."}
              </td>
            </tr>
          )}
          {visible.map((row) => (
            <tr
              key={rowKey(row)}
              className={`${onRowClick ? "row-clickable" : ""} ${rowClassName?.(row) ?? ""}`}
              onClick={onRowClick ? () => onRowClick(row) : undefined}
            >
              {columns.map((c) => {
                const value = c.cell ? c.cell(row) : ((row as unknown as Record<string, ReactNode>)[c.key] ?? "");
                return (
                  <td key={c.key} className={c.align === "num" ? "num" : ""}>
                    {value}
                  </td>
                );
              })}
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  );
}
