import {
  createContext,
  ReactNode,
  useCallback,
  useContext,
  useEffect,
  useState,
} from 'react';
import { onAuthStateChanged, signOut, type User as FbUser } from 'firebase/auth';
import { auth } from './firebase';
import { apiMe, type User } from './api';

type AuthState = {
  fbUser: FbUser | null;
  user: User | null;
  loading: boolean;
  error: string | null;
  reload: () => Promise<void>;
  logout: () => Promise<void>;
};

// Keep one context object across Vite hot updates. When api.ts or firebase.ts
// changes, this module re-runs; a new context would no longer match the
// AuthProvider already mounted by main.tsx, and useAuth would throw.
const Ctx: React.Context<AuthState | null> =
  import.meta.hot?.data.authCtx ?? createContext<AuthState | null>(null);
if (import.meta.hot) import.meta.hot.data.authCtx = Ctx;

export function AuthProvider({ children }: { children: ReactNode }) {
  const [fbUser, setFbUser] = useState<FbUser | null>(null);
  const [authReady, setAuthReady] = useState(false);
  const [user, setUser] = useState<User | null>(null);
  const [profileLoading, setProfileLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);

  const fetchProfile = useCallback(async () => {
    setProfileLoading(true);
    setError(null);
    try {
      const me = await apiMe();
      setUser(me);
    } catch (err: any) {
      setError(err?.message ?? 'Failed to fetch profile');
      setUser(null);
    } finally {
      setProfileLoading(false);
    }
  }, []);

  useEffect(() => {
    return onAuthStateChanged(auth, u => {
      setFbUser(u);
      setAuthReady(true);
      if (!u) {
        setUser(null);
        setError(null);
        setProfileLoading(false);
      }
    });
  }, []);

  useEffect(() => {
    if (fbUser) fetchProfile();
  }, [fbUser, fetchProfile]);

  const value: AuthState = {
    fbUser,
    user,
    loading: !authReady || profileLoading,
    error,
    reload: fetchProfile,
    logout: () => signOut(auth),
  };

  return <Ctx.Provider value={value}>{children}</Ctx.Provider>;
}

export function useAuth() {
  const ctx = useContext(Ctx);
  if (!ctx) throw new Error('useAuth must be used within AuthProvider');
  return ctx;
}

/**
 * A user can use the business panel if:
 *   - their account was approved by an admin (companyApproved === true), OR
 *   - they have one of the company-side roles assigned (courier/manager/controller),
 *     which means a company owner brought them in.
 * Banned accounts are rejected unconditionally.
 */
export function hasCompanyAccess(u: User | null): boolean {
  if (!u) return false;
  if (u.banned) return false;
  if (u.companyApproved) return true;
  const roles = u.roles ?? [];
  return roles.some(r => ['owner', 'manager', 'controller', 'courier'].includes(r));
}
