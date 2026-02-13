import { ChevronRight, ArrowUp } from "lucide-react";
import { useTeamStore } from "../stores/teamStore";

export default function Breadcrumb() {
  const breadcrumbs = useTeamStore((s) => s.breadcrumbs);
  const navigateToRoot = useTeamStore((s) => s.navigateToRoot);
  const navigateToBreadcrumb = useTeamStore((s) => s.navigateToBreadcrumb);
  const navigateBack = useTeamStore((s) => s.navigateBack);

  const atRoot = breadcrumbs.length === 0;

  return (
    <div
      className="flex items-center gap-1 px-4 py-1.5 text-sm border-b overflow-x-auto"
      style={{
        background: "var(--bg-secondary)",
        borderColor: "var(--border)",
        minHeight: 36,
      }}
    >
      {atRoot ? (
        <span
          className="px-2 py-0.5 text-xs font-medium whitespace-nowrap"
          style={{ color: "var(--accent)" }}
        >
          All
        </span>
      ) : (
        <button
          className="btn-secondary px-2 py-0.5 text-xs whitespace-nowrap"
          onClick={navigateToRoot}
        >
          All
        </button>
      )}

      {breadcrumbs.map((crumb, index) => {
        const isLast = index === breadcrumbs.length - 1;
        return (
          <div key={crumb.id} className="flex items-center gap-1">
            <ChevronRight
              size={14}
              style={{ color: "var(--text-secondary)", flexShrink: 0 }}
            />
            {isLast ? (
              <span
                className="px-2 py-0.5 text-xs font-medium rounded whitespace-nowrap"
                style={{ color: "var(--accent)" }}
              >
                {crumb.title}
              </span>
            ) : (
              <button
                className="btn-secondary px-2 py-0.5 text-xs whitespace-nowrap"
                onClick={() => navigateToBreadcrumb(crumb.id)}
              >
                {crumb.title}
              </button>
            )}
          </div>
        );
      })}

      {!atRoot && (
        <button
          className="btn-secondary p-1 ml-2"
          onClick={navigateBack}
          title="Go up one level"
        >
          <ArrowUp size={14} />
        </button>
      )}
    </div>
  );
}
