/**
 * Gift claim links open the buyer app, where people sign in and take the gift.
 * VITE_USER_APP_URL points to it (default: the local buyer app).
 */
const USER_APP = ((import.meta.env.VITE_USER_APP_URL as string) || 'http://localhost:3002/').replace(/\/?$/, '/');
export const giftUrl = (code: string) => `${USER_APP}?gift=${encodeURIComponent(code)}`;
