import { useEffect, useState } from 'react';
import { Icon } from '../icons';
import { apiChangePassword, apiUpdateProfile, apiUploadAvatar } from '../api';
import { useAuth } from '../auth';

export default function ProfilePage() {
  const { user, reload } = useAuth();

  function ProfileForm() {
    const [name, setName] = useState(user!.name);
    const [username, setUsername] = useState(user!.username);
    const [bio, setBio] = useState(user!.bio ?? '');
    const [location, setLocation] = useState(user!.location ?? '');
    const [deliveryAddress, setDeliveryAddress] = useState(user!.deliveryAddress ?? '');
    const [avatarFile, setAvatarFile] = useState<File | null>(null);
    const [avatarPreview, setAvatarPreview] = useState('');
    useEffect(() => {
      if (!avatarFile) { setAvatarPreview(''); return; }
      const url = URL.createObjectURL(avatarFile);
      setAvatarPreview(url);
      return () => URL.revokeObjectURL(url);
    }, [avatarFile]);
    const [busy, setBusy] = useState(false);
    const [msg, setMsg] = useState<string | null>(null);
    const [err, setErr] = useState<string | null>(null);

    const submit = async (e: React.FormEvent) => {
      e.preventDefault();
      setBusy(true);
      setErr(null);
      setMsg(null);
      try {
        if (avatarFile) {
          await apiUploadAvatar(user!.uid, avatarFile);
        }
        await apiUpdateProfile(user!.uid, {
          name: name.trim(),
          username: username.trim(),
          bio: bio.trim(),
          location: location.trim(),
          deliveryAddress: deliveryAddress.trim(),
        });
        await reload();
        setMsg('Зміни збережено.');
        setAvatarFile(null);
      } catch (e: any) {
        setErr(e?.message ?? 'Не вдалося зберегти');
      } finally {
        setBusy(false);
      }
    };

    return (
      <form onSubmit={submit} className="card">
        <h3>Профіль</h3>
        <p className="sub" style={{ marginBottom: 16 }}>Так тебе бачать покупці в застосунку Marki.</p>

        <div style={{ display: 'flex', alignItems: 'center', gap: 16, marginBottom: 18 }}>
          {avatarPreview || user!.avatar ? (
            <img src={avatarPreview || user!.avatar} alt="Аватар" style={{ width: 72, height: 72, borderRadius: '50%', objectFit: 'cover' }} />
          ) : (
            <span style={{ width: 72, height: 72, borderRadius: '50%', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', background: 'var(--primary-soft)', color: 'var(--primary)', fontSize: 28, fontWeight: 700 }}>
              {(user!.name || 'M').slice(0, 1).toUpperCase()}
            </span>
          )}
          <div>
            <label className="file-pick btn">
              <Icon.Upload /> {avatarPreview ? 'Обрати інше фото' : 'Змінити фото'}
              <input type="file" accept="image/*" onChange={e => setAvatarFile(e.target.files?.[0] ?? null)} />
            </label>
            <div className="muted" style={{ fontSize: 11, marginTop: 6 }}>
              {avatarFile ? `${avatarFile.name} — збережеться після «Зберегти»` : 'JPG або PNG, до 5 МБ'}
            </div>
          </div>
        </div>

        <div className="field">
          <label>Ім’я</label>
          <input value={name} onChange={e => setName(e.target.value)} />
        </div>
        <div className="field">
          <label>Нікнейм</label>
          <input value={username} onChange={e => setUsername(e.target.value)} placeholder="напр. water_ua" />
        </div>
        <div className="field">
          <label>Про себе</label>
          <textarea value={bio} onChange={e => setBio(e.target.value)} placeholder="Кілька слів про бренд" />
        </div>
        <div className="field">
          <label>Місто</label>
          <input value={location} onChange={e => setLocation(e.target.value)} placeholder="Київ, Україна" />
        </div>
        <div className="field">
          <label>Адреса доставки за замовчуванням</label>
          <input value={deliveryAddress} onChange={e => setDeliveryAddress(e.target.value)} />
        </div>

        {msg && <div className="success-banner" style={{ marginBottom: 12 }}>{msg}</div>}
        {err && <div className="error-banner">{err}</div>}

        <button type="submit" className="btn btn-primary btn-block" disabled={busy}>
          {busy ? 'Збереження…' : 'Зберегти зміни'}
        </button>
      </form>
    );
  }

  function SecurityCard() {
    const [pw, setPw] = useState('');
    const [busy, setBusy] = useState(false);
    const [msg, setMsg] = useState<string | null>(null);
    const [err, setErr] = useState<string | null>(null);

    const submit = async () => {
      if (pw.length < 8) {
        setErr('Пароль має містити щонайменше 8 символів.');
        return;
      }
      setBusy(true);
      setErr(null);
      setMsg(null);
      try {
        await apiChangePassword(user!.uid, pw);
        setMsg('Пароль змінено. Наступного разу входь з новим паролем.');
        setPw('');
      } catch (e: any) {
        setErr(e?.message ?? 'Не вдалося змінити пароль');
      } finally {
        setBusy(false);
      }
    };

    return (
      <div className="card">
        <h3>Безпека</h3>
        <p className="sub" style={{ marginBottom: 14 }}>Зміна пароля для входу через email.</p>
        <div className="field">
          <label>Новий пароль</label>
          <input type="password" value={pw} onChange={e => setPw(e.target.value)} placeholder="Щонайменше 8 символів" autoComplete="new-password" />
        </div>
        {msg && <div className="success-banner" style={{ marginBottom: 12 }}>{msg}</div>}
        {err && <div className="error-banner">{err}</div>}
        <button className="btn btn-primary btn-block" disabled={busy || !pw} onClick={submit}>
          {busy ? 'Оновлення…' : 'Змінити пароль'}
        </button>
      </div>
    );
  }

  function CompanyInfoCard() {
    if (!user!.companyApproved) return null;
    return (
      <div className="card">
        <h3>Компанія</h3>
        <p className="sub" style={{ marginBottom: 14 }}>Компанію перевірено — покупці бачать позначку «Перевірений бренд».</p>
        <Row label="Назва" value={user!.companyName} />
        <Row label="Реєстраційний номер" value={user!.registrationNumber} />
        <Row label="Контактний email" value={user!.contactEmail} />
        <Row label="Опис" value={user!.businessDescription} multiline />
        <div style={{ marginTop: 12 }}>
          <span className="badge badge-success">✓ Перевірено</span>
        </div>
      </div>
    );
  }

  function Row({ label, value, multiline }: { label: string; value?: string; multiline?: boolean }) {
    if (!value) return null;
    return (
      <div style={{ marginBottom: 10 }}>
        <div className="sub" style={{ fontSize: 11, textTransform: 'uppercase', letterSpacing: 0.04 }}>{label}</div>
        <div style={{ fontSize: 13, whiteSpace: multiline ? 'pre-wrap' : 'normal' }}>{value}</div>
      </div>
    );
  }

  if (!user) return null;

  return (
    <div>
    <div className="page-head">
      <div>
        <h2>Профіль</h2>
        <p>Дані компанії, аватар і безпека.</p>
      </div>
    </div>
    <div className="grid grid-2" style={{ alignItems: 'start' }}>
      <ProfileForm />
      <div>
        <SecurityCard />
        <div style={{ height: 12 }} />
        <CompanyInfoCard />
      </div>
    </div>
    </div>
  );
}
