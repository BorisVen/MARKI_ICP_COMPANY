import { useState } from 'react';
import {
  GoogleAuthProvider,
  signInWithEmailAndPassword,
  signInWithPopup,
} from 'firebase/auth';
import { auth } from './firebase';

/** Map raw Firebase error codes to friendly Ukrainian messages. */
function authErrorMessage(err: any): string {
  const code: string = err?.code ?? '';
  console.warn('[Login] Firebase auth error:', code, err?.message);
  switch (code) {
    case 'auth/invalid-credential':
    case 'auth/wrong-password':
    case 'auth/user-not-found':
      // Firebase returns the same code for a wrong password, an unknown email and
      // an account that only has Google sign-in, so show the code for support.
      return `Невірний email або пароль (${code}). Якщо акаунт створено через Google — увійди кнопкою Google.`;
    case 'auth/invalid-email':
      return 'Некоректний email.';
    case 'auth/user-disabled':
      return 'Обліковий запис вимкнено.';
    case 'auth/too-many-requests':
      return 'Забагато спроб. Спробуй пізніше.';
    case 'auth/network-request-failed':
      return 'Проблема з мережею. Перевір з’єднання.';
    case 'auth/popup-closed-by-user':
    case 'auth/cancelled-popup-request':
      return 'Вікно входу було закрито.';
    case 'auth/popup-blocked':
      return 'Браузер заблокував вікно входу. Дозволь спливаючі вікна для цього сайту.';
    case 'auth/unauthorized-domain':
      return `Домен ${window.location.hostname} не доданий у Firebase Console → Authentication → Settings → Authorized domains.`;
    case 'auth/operation-not-allowed':
      return 'Цей спосіб входу вимкнено у Firebase Console.';
    case 'auth/invalid-api-key':
    case 'auth/api-key-not-valid.-please-pass-a-valid-api-key.':
      return 'Неправильний VITE_FIREBASE_API_KEY у .env.local.';
    default:
      return code ? `Не вдалося увійти (${code}).` : err?.message ?? 'Не вдалося увійти.';
  }
}

export default function Login() {
  const [email, setEmail] = useState('');
  const [password, setPassword] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const onGoogle = async () => {
    setBusy(true);
    setError(null);
    try {
      await signInWithPopup(auth, new GoogleAuthProvider());
    } catch (err: any) {
      setError(authErrorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  const onSubmit = async (e: React.FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      await signInWithEmailAndPassword(auth, email.trim(), password.trim());
    } catch (err: any) {
      setError(authErrorMessage(err));
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="login-shell">
      <div className="login-card">
        <h2>Marki для бізнесу</h2>
        <p className="sub">Керуй своїм бізнесом із браузера.</p>

        <button className="btn btn-block" type="button" onClick={onGoogle} disabled={busy}>
          {busy ? 'Зачекай…' : 'Продовжити з Google'}
        </button>

        <div className="divider">АБО EMAIL</div>

        <form onSubmit={onSubmit}>
          <div className="field">
            <label>Email</label>
            <input
              type="email"
              value={email}
              onChange={e => setEmail(e.target.value)}
              autoComplete="email"
            />
          </div>
          <div className="field">
            <label>Пароль</label>
            <input
              type="password"
              value={password}
              onChange={e => setPassword(e.target.value)}
              autoComplete="current-password"
            />
          </div>
          <button
            type="submit"
            className="btn btn-primary btn-block"
            disabled={busy || !email || !password}
          >
            {busy ? 'Входимо…' : 'Увійти'}
          </button>
          {error && <div className="error-banner" style={{ marginTop: 14 }}>{error}</div>}
        </form>
      </div>
    </div>
  );
}
