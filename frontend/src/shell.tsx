import type { ComponentChildren } from 'preact';
import { useCallback, useEffect, useState } from 'preact/hooks';
import {
  destinationForSetupStatus,
  sessionExpiredView,
  validateSetupCandidate,
  viewAfterSetupConflict,
  type AppView,
  type LoginNoticeId,
} from './auth-flow';
import {
  ApiError,
  getSetupStatus,
  login,
  logout,
  setupOwner,
} from './api';
import { Dashboard } from './app';
import { t } from './i18n';
import {
  applyThemePreference,
  readThemePreference,
  saveThemePreference,
  themeOptions,
  type ThemePreference,
} from './theme';
import type { AuthSession, SetupStatus } from './types';

// HTML maxlength counts UTF-16 code units. These admission ceilings preserve every
// server-valid Unicode value; validateSetupCandidate applies the canonical code-point
// and UTF-8 byte limits before any request is sent.
const DISPLAY_NAME_INPUT_MAX_CODE_UNITS = 512;
const PASSWORD_INPUT_MAX_CODE_UNITS = 1_024;

function errorMessage(error: unknown): string {
  if (error instanceof ApiError) {
    return error.message;
  }
  return t('auth.error.fallback');
}

function useAuthTheme(): [ThemePreference, (theme: ThemePreference) => void] {
  const [theme, setTheme] = useState<ThemePreference>(readThemePreference);

  useEffect(() => {
    saveThemePreference(theme);
    const colorPreference = window.matchMedia('(prefers-color-scheme: light)');
    const apply = (): void => {
      applyThemePreference(theme, colorPreference.matches);
    };
    apply();
    if (theme === 'system') {
      colorPreference.addEventListener('change', apply);
    }
    return () => colorPreference.removeEventListener('change', apply);
  }, [theme]);

  return [theme, setTheme];
}

function AuthBrand() {
  return (
    <div class="brand" aria-label="Helix">
      <span class="brand__mark" aria-hidden="true">
        <span />
        <span />
      </span>
      <span class="brand__wordmark">HELIX</span>
    </div>
  );
}

function AuthFrame({ children }: { children: ComponentChildren }) {
  const [theme, setTheme] = useAuthTheme();
  return (
    <main class="auth-page" id="main-content" tabIndex={-1}>
      <a class="skip-link" href="#main-content">{t('auth.skipToForm')}</a>
      <header class="auth-page__header">
        <AuthBrand />
        <label class="theme-selector">
          <span class="sr-only">{t('common.colorTheme')}</span>
          <span class="theme-selector__swatch" aria-hidden="true" />
          <select
            value={theme}
            onChange={(event) =>
              setTheme(event.currentTarget.value as ThemePreference)
            }
            aria-label={t('common.colorTheme')}
          >
            {themeOptions.map((option) => (
              <option key={option.value} value={option.value}>
                {t(option.labelId)}
              </option>
            ))}
          </select>
        </label>
      </header>
      <div class="auth-page__glow" aria-hidden="true" />
      {children}
    </main>
  );
}

function LoadingView() {
  return (
    <AuthFrame>
      <section class="auth-card auth-card--status" id="auth-card" aria-live="polite">
        <span class="auth-card__spinner" aria-hidden="true" />
        <span class="eyebrow">{t('auth.loading.eyebrow')}</span>
        <h1>{t('auth.loading.title')}</h1>
        <p>{t('auth.loading.detail')}</p>
      </section>
    </AuthFrame>
  );
}

function ErrorView({ message, onRetry }: { message: string; onRetry: () => void }) {
  return (
    <AuthFrame>
      <section class="auth-card auth-card--status" id="auth-card">
        <span class="eyebrow">{t('auth.unavailable.eyebrow')}</span>
        <h1>{t('auth.unavailable.title')}</h1>
        <p role="alert">{message}</p>
        <button class="primary-button" type="button" onClick={onRetry}>
          {t('common.tryAgain')}
        </button>
      </section>
    </AuthFrame>
  );
}

function SetupView({
  status,
  onAuthenticated,
  onSetupConflict,
}: {
  status: SetupStatus;
  onAuthenticated: (session: AuthSession) => void;
  onSetupConflict: () => Promise<void>;
}) {
  const [bootstrapToken, setBootstrapToken] = useState('');
  const [loginName, setLoginName] = useState('');
  const [displayName, setDisplayName] = useState('');
  const [password, setPassword] = useState('');
  const [confirmation, setConfirmation] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const clearSecrets = (): void => {
    setBootstrapToken('');
    setPassword('');
    setConfirmation('');
  };

  const submit = async (event: Event): Promise<void> => {
    event.preventDefault();
    if (submitting) {
      return;
    }
    const validationError = validateSetupCandidate({
      bootstrapToken,
      loginName,
      displayName,
      password,
      confirmation,
    });
    if (validationError !== null) {
      setError(t(validationError));
      clearSecrets();
      return;
    }

    setSubmitting(true);
    setError(null);
    try {
      const session = await setupOwner({
        bootstrapToken,
        loginName,
        displayName,
        password,
      });
      onAuthenticated(session);
    } catch (requestError) {
      if (requestError instanceof ApiError && requestError.status === 409) {
        try {
          await onSetupConflict();
        } catch (recoveryError) {
          setError(errorMessage(recoveryError));
        }
      } else {
        setError(errorMessage(requestError));
      }
    } finally {
      clearSecrets();
      setSubmitting(false);
    }
  };

  return (
    <AuthFrame>
      <section class="auth-card" id="auth-card" aria-labelledby="setup-title">
        <div class="auth-card__heading">
          <span class="eyebrow">{t('auth.setup.eyebrow')}</span>
          <h1 id="setup-title">{t('auth.setup.title')}</h1>
          <p>
            {t('auth.setup.instructionsBeforeCommand')}{' '}
            <code>helixctl setup-token</code>{' '}
            {t('auth.setup.instructionsAfterCommand')}
          </p>
        </div>
        {!status.bootstrapAvailable && (
          <div class="form-notice" role="status">
            {t('auth.setup.noToken')}
          </div>
        )}
        <form class="auth-form" noValidate onSubmit={(event) => void submit(event)}>
          <label>
            <span>{t('auth.setup.token')}</span>
            <input
              type="password"
              value={bootstrapToken}
              onInput={(event) => setBootstrapToken(event.currentTarget.value.trim())}
              minLength={43}
              maxLength={43}
              pattern="[A-Za-z0-9_-]{43}"
              autoComplete="off"
              spellcheck={false}
              required
            />
          </label>
          <div class="auth-form__split">
            <label>
              <span>{t('auth.setup.loginName')}</span>
              <input
                type="text"
                value={loginName}
                onInput={(event) =>
                  setLoginName(event.currentTarget.value.toLowerCase())
                }
                minLength={3}
                maxLength={64}
                pattern="[a-z0-9](?:[a-z0-9._-]{1,62}[a-z0-9])?"
                autoComplete="username"
                autocapitalize="none"
                spellcheck={false}
                required
              />
            </label>
            <label>
              <span>{t('auth.setup.displayName')}</span>
              <input
                type="text"
                value={displayName}
                onInput={(event) => setDisplayName(event.currentTarget.value)}
                maxLength={DISPLAY_NAME_INPUT_MAX_CODE_UNITS}
                autoComplete="name"
                aria-describedby="setup-display-name-help"
                required
              />
              <small id="setup-display-name-help">
                {t('auth.setup.displayNameHelp')}
              </small>
            </label>
          </div>
          <label>
            <span>{t('auth.setup.password')}</span>
            <input
              type="password"
              value={password}
              onInput={(event) => setPassword(event.currentTarget.value)}
              minLength={15}
              maxLength={PASSWORD_INPUT_MAX_CODE_UNITS}
              autoComplete="new-password"
              aria-describedby="setup-password-help"
              required
            />
            <small id="setup-password-help">{t('auth.setup.passwordHelp')}</small>
          </label>
          <label>
            <span>{t('auth.setup.confirmPassword')}</span>
            <input
              type="password"
              value={confirmation}
              onInput={(event) => setConfirmation(event.currentTarget.value)}
              minLength={15}
              maxLength={PASSWORD_INPUT_MAX_CODE_UNITS}
              autoComplete="new-password"
              required
            />
          </label>
          {error !== null && <div class="form-error" role="alert">{error}</div>}
          <button
            class="primary-button"
            type="submit"
            aria-busy={submitting}
            aria-disabled={submitting}
          >
            {submitting ? t('auth.setup.submitting') : t('auth.setup.submit')}
          </button>
        </form>
        <p class="auth-card__footnote">
          {t('auth.setup.footnote')}
        </p>
      </section>
    </AuthFrame>
  );
}

function LoginView({
  notice,
  onAuthenticated,
}: {
  notice: LoginNoticeId | null;
  onAuthenticated: (session: AuthSession) => void;
}) {
  const [loginName, setLoginName] = useState('');
  const [password, setPassword] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [submitting, setSubmitting] = useState(false);

  const submit = async (event: Event): Promise<void> => {
    event.preventDefault();
    if (submitting) {
      return;
    }
    setSubmitting(true);
    setError(null);
    try {
      onAuthenticated(await login({ loginName, password }));
    } catch (requestError) {
      setError(errorMessage(requestError));
    } finally {
      setPassword('');
      setSubmitting(false);
    }
  };

  return (
    <AuthFrame>
      <section class="auth-card auth-card--login" id="auth-card" aria-labelledby="login-title">
        <div class="auth-card__heading">
          <span class="eyebrow">{t('auth.login.eyebrow')}</span>
          <h1 id="login-title">{t('auth.login.title')}</h1>
          <p>{t('auth.login.detail')}</p>
        </div>
        {notice !== null && <div class="form-notice" role="status">{t(notice)}</div>}
        <form class="auth-form" onSubmit={(event) => void submit(event)}>
          <label>
            <span>{t('auth.login.loginName')}</span>
            <input
              type="text"
              value={loginName}
              onInput={(event) =>
                setLoginName(event.currentTarget.value.toLowerCase())
              }
              maxLength={64}
              autoComplete="username"
              autocapitalize="none"
              spellcheck={false}
              autofocus
              required
            />
          </label>
          <label>
            <span>{t('auth.login.password')}</span>
            <input
              type="password"
              value={password}
              onInput={(event) => setPassword(event.currentTarget.value)}
              maxLength={1024}
              autoComplete="current-password"
              required
            />
          </label>
          {error !== null && <div class="form-error" role="alert">{error}</div>}
          <button
            class="primary-button"
            type="submit"
            aria-busy={submitting}
            aria-disabled={submitting}
          >
            {submitting ? t('auth.login.submitting') : t('auth.login.submit')}
          </button>
        </form>
        <p class="auth-card__footnote">
          {t('auth.login.footnote')}
        </p>
      </section>
    </AuthFrame>
  );
}

export function App() {
  const [view, setView] = useState<AppView>({ kind: 'loading' });
  const [bootAttempt, setBootAttempt] = useState(0);

  useEffect(() => {
    const controller = new AbortController();
    const boot = async (): Promise<void> => {
      setView({ kind: 'loading' });
      try {
        const status = await getSetupStatus(controller.signal);
        const destination = destinationForSetupStatus(status);
        setView(destination);
      } catch (error) {
        if (!controller.signal.aborted) {
          setView({ kind: 'error', message: errorMessage(error) });
        }
      }
    };
    void boot();
    return () => controller.abort();
  }, [bootAttempt]);

  const authenticated = useCallback((session: AuthSession): void => {
    setView({ kind: 'dashboard', session });
  }, []);
  const recoverSetupConflict = useCallback(async (): Promise<void> => {
    const status = await getSetupStatus();
    setView(viewAfterSetupConflict(status));
  }, []);
  const sessionExpired = useCallback(
    (): void => setView(sessionExpiredView()),
    [],
  );

  if (view.kind === 'loading') {
    return <LoadingView />;
  }
  if (view.kind === 'error') {
    return (
      <ErrorView
        message={view.message}
        onRetry={() => setBootAttempt((attempt) => attempt + 1)}
      />
    );
  }
  if (view.kind === 'setup') {
    return (
      <SetupView
        status={view.status}
        onAuthenticated={authenticated}
        onSetupConflict={recoverSetupConflict}
      />
    );
  }
  if (view.kind === 'login') {
    return <LoginView notice={view.notice} onAuthenticated={authenticated} />;
  }

  const currentSession = view.session;
  const signOut = async (): Promise<void> => {
    try {
      await logout(currentSession.csrfToken);
    } catch (error) {
      if (!(error instanceof ApiError) || error.status !== 401) {
        throw error;
      }
    }
    setView({ kind: 'login', notice: null });
  };
  return (
    <Dashboard
      user={currentSession.user}
      csrfToken={currentSession.csrfToken}
      onSessionExpired={sessionExpired}
      onLogout={signOut}
    />
  );
}
