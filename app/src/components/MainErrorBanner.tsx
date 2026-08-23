interface MainErrorBannerProps {
  message: string;
  onDismiss: () => void;
}

export function MainErrorBanner({ message, onDismiss }: MainErrorBannerProps) {
  return (
    <div
      role="alert"
      aria-atomic="true"
      data-testid="main-error-banner"
      className="main-error-banner"
    >
      <p className="min-w-0 flex-1 break-words text-xs leading-5 text-error">
        {message}
      </p>
      <button
        type="button"
        onClick={onDismiss}
        aria-label="Dismiss error"
        className="ui-icon-button -mr-1 -mt-1 h-7 w-7 shrink-0 text-on-surface-variant hover:bg-surface-container-high hover:text-on-surface focus:outline-none focus-visible:ring-2 focus-visible:ring-primary"
      >
        <svg className="h-3.5 w-3.5" viewBox="0 0 20 20" fill="none" stroke="currentColor" aria-hidden="true">
          <path d="M5 5l10 10M15 5L5 15" strokeWidth="1.8" strokeLinecap="round" />
        </svg>
      </button>
    </div>
  );
}
