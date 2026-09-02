import type { SessionNotice } from "../types/session";

interface SessionNoticeCardProps {
  notice: SessionNotice;
}

export function SessionNoticeCard({ notice }: SessionNoticeCardProps) {
  return (
    <div
      id="session-notice-card"
      data-testid={`session-notice-card-${notice.level}`}
      className={`session-notice-card session-notice-card--${notice.level}`}
      role="alert"
    >
      <div className="session-notice-card__body">
        <div className="session-notice-card__title">{notice.title}</div>
        <div className="session-notice-card__message">{notice.message}</div>
      </div>
      {!!notice.actions?.length && (
        <div className="session-notice-card__actions">
          {notice.actions.map((action) => (
            <button
              key={action.id}
              id={`session-notice-action-${action.id}`}
              data-testid={`session-notice-action-${action.id}`}
              type="button"
              className="session-notice-card__action"
              onClick={action.onClick}
              aria-label={action.label}
            >
              {action.label}
            </button>
          ))}
        </div>
      )}
    </div>
  );
}
