import { useAuth } from './auth';
import { apiRequestApproval } from './api';
import { useState } from 'react';

export default function AccessDenied() {
  const { user, logout, reload } = useAuth();
  const [companyName, setCompanyName] = useState('');
  const [regNumber, setRegNumber] = useState('');
  const [contactEmail, setContactEmail] = useState('');
  const [description, setDescription] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [done, setDone] = useState(false);

  if (!user) return null;

  if (user.banned) {
    return (
      <div className="login-shell">
        <div className="login-card">
          <h2>Акаунт заблоковано</h2>
          <p className="sub">
            Доступ до панелі призупинено.
            {user.banReason ? ` Причина: ${user.banReason}` : ''}
          </p>
          <button className="btn btn-block" onClick={logout}>Вийти</button>
        </div>
      </div>
    );
  }

  if (user.pendingApproval) {
    return (
      <div className="login-shell">
        <div className="login-card">
          <h2>Заявка на розгляді</h2>
          <p className="sub">
            Ми отримали твою заявку. Щойно адмін її підтвердить — тут з’явиться панель керування.
          </p>
          <button className="btn btn-block" style={{ marginBottom: 8 }} onClick={reload}>
            Перевірити статус
          </button>
          <button className="btn btn-block" onClick={logout}>Вийти</button>
        </div>
      </div>
    );
  }

  if (done) {
    return (
      <div className="login-shell">
        <div className="login-card">
          <h2>Заявку надіслано</h2>
          <p className="sub">
            Адмін розгляне її найближчим часом. Можеш оновити статус кнопкою нижче.
          </p>
          <button className="btn btn-block" onClick={reload}>Перевірити статус</button>
        </div>
      </div>
    );
  }

  const submit = async (e: React.FormEvent) => {
    e.preventDefault();
    if (!companyName.trim() || !regNumber.trim() || !contactEmail.trim()) {
      setError('Заповни обов’язкові поля.');
      return;
    }
    setBusy(true);
    setError(null);
    try {
      await apiRequestApproval(user.uid, {
        companyName: companyName.trim(),
        registrationNumber: regNumber.trim(),
        contactEmail: contactEmail.trim(),
        description: description.trim(),
      });
      setDone(true);
      reload();
    } catch (err: any) {
      setError(err?.message ?? 'Submit failed');
    } finally {
      setBusy(false);
    }
  };

  return (
    <div className="login-shell">
      <div className="login-card">
        <h2>Підключи компанію</h2>
        <p className="sub">
          Щоб відкрити бізнес-панель, надішли заявку. Адмін підтвердить її — і доступ з’явиться автоматично.
        </p>

        <form onSubmit={submit}>
          <div className="field">
            <label>Назва компанії *</label>
            <input value={companyName} onChange={e => setCompanyName(e.target.value)} />
          </div>
          <div className="field">
            <label>Реєстраційний номер *</label>
            <input value={regNumber} onChange={e => setRegNumber(e.target.value)} />
          </div>
          <div className="field">
            <label>Контактний email *</label>
            <input type="email" value={contactEmail} onChange={e => setContactEmail(e.target.value)} />
          </div>
          <div className="field">
            <label>Опис</label>
            <textarea value={description} onChange={e => setDescription(e.target.value)} />
          </div>
          <button type="submit" className="btn btn-primary btn-block" disabled={busy}>
            {busy ? 'Відправлення…' : 'Надіслати заявку'}
          </button>
          {error && <div className="error-banner" style={{ marginTop: 14 }}>{error}</div>}
          <button type="button" className="btn btn-block" style={{ marginTop: 8 }} onClick={logout}>
            Вийти
          </button>
        </form>
      </div>
    </div>
  );
}
