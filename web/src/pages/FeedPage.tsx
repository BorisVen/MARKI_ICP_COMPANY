import { useCallback, useEffect, useState } from 'react';
import {
  apiAddComment,
  apiCreatePost,
  apiDeletePost,
  apiGetPosts,
  apiLikePost,
  type Post,
} from '../api';
import { useAuth } from '../auth';
import { Icon } from '../icons';
import EmptyState from '../EmptyState';
import { fmtAgo, fmtDateTime, fmtPrice, plural } from '../format';

export default function FeedPage() {
  const { user } = useAuth();
  const [posts, setPosts] = useState<Post[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [newText, setNewText] = useState('');
  const [posting, setPosting] = useState(false);

  const reload = useCallback(async () => {
    setLoading(true);
    setError(null);
    try {
      setPosts(await apiGetPosts());
    } catch (e: any) {
      setError(e?.message ?? 'Не вдалося завантажити стрічку');
    } finally {
      setLoading(false);
    }
  }, []);

  useEffect(() => { reload(); }, [reload]);

  const createPost = async () => {
    if (!newText.trim()) return;
    setPosting(true);
    try {
      await apiCreatePost({ text: newText.trim() });
      setNewText('');
      await reload();
    } catch (e: any) {
      alert(e?.message ?? 'Не вдалося опублікувати');
    } finally {
      setPosting(false);
    }
  };

  const like = async (id: string) => {
    try { await apiLikePost(id); await reload(); } catch {}
  };

  const remove = async (id: string) => {
    if (!confirm('Видалити пост?')) return;
    try { await apiDeletePost(id); await reload(); } catch (e: any) { alert(e?.message ?? 'Не вдалося виконати дію'); }
  };

  const comment = async (id: string, text: string) => {
    if (!text.trim()) return;
    try { await apiAddComment(id, text.trim()); await reload(); } catch (e: any) { alert(e?.message ?? 'Не вдалося виконати дію'); }
  };

  return (
    <div>
      <div className="page-head">
        <div>
          <h2>Стрічка</h2>
          <p>Новини компаній і нові NFT у спільноті.</p>
        </div>
        <button className="btn" onClick={reload} title="Оновити"><Icon.Refresh /></button>
      </div>

      <div className="card" style={{ marginBottom: 20 }}>
        <div className="field" style={{ marginBottom: 10 }}>
          <textarea
            placeholder="Розкажи покупцям про новинку, акцію чи подію…"
            value={newText}
            onChange={e => setNewText(e.target.value)}
            style={{ minHeight: 60 }}
          />
        </div>
        <button className="btn btn-primary" onClick={createPost} disabled={posting || !newText.trim()}>
          {posting ? 'Публікація…' : 'Опублікувати'}
        </button>
      </div>

      {error && <div className="error-banner">{error}</div>}

      {loading ? (
        <div className="spinner">Завантаження…</div>
      ) : posts.length === 0 ? (
        <EmptyState icon="📰" title="Стрічка поки порожня" text="Напиши перший пост або випусти NFT — він з’явиться тут." />
      ) : (
        posts.map(p => (
          <PostCard
            key={p.id}
            post={p}
            currentUid={user?.uid}
            onLike={() => like(p.id)}
            onDelete={() => remove(p.id)}
            onComment={(text) => comment(p.id, text)}
          />
        ))
      )}
    </div>
  );
}

function PostCard({
  post,
  currentUid,
  onLike,
  onDelete,
  onComment,
}: {
  post: Post;
  currentUid?: string;
  onLike: () => void;
  onDelete: () => void;
  onComment: (text: string) => void;
}) {
  const [commentText, setCommentText] = useState('');
  const isMine = post.userId === currentUid;
  const liked = (post.likedBy || []).includes(currentUid || '');

  return (
    <div className="card" style={{ marginBottom: 12 }}>
      <div style={{ display: 'flex', justifyContent: 'space-between' }}>
        <div>
          <div style={{ display: 'flex', gap: 10, alignItems: 'center' }}>
            {post.authorAvatar
              ? <img src={post.authorAvatar} alt="" style={{ width: 34, height: 34, borderRadius: '50%', objectFit: 'cover' }} />
              : <span className="avatar-letter" style={{ width: 34, height: 34, borderRadius: '50%', display: 'inline-flex', alignItems: 'center', justifyContent: 'center', background: 'var(--primary-soft)', color: 'var(--primary)', fontWeight: 600 }}>{(post.authorName || 'M').slice(0, 1).toUpperCase()}</span>}
            <div>
              <h3 style={{ fontSize: 14, margin: 0 }}>{post.authorName || (isMine ? 'Ви' : 'Учасник Marki')}</h3>
              <div className="sub" style={{ fontSize: 11 }} title={fmtDateTime(post.createdAt)}>{fmtAgo(post.createdAt)}</div>
            </div>
          </div>
        </div>
        {isMine && (
          <button className="btn ghost-danger" onClick={onDelete} title="Видалити пост"><Icon.Trash /></button>
        )}
      </div>

      {post.text && <p style={{ marginTop: 10, fontSize: 14 }}>{post.text}</p>}

      {(post.nftTitle || post.title) && <div style={{ marginTop: 10, fontWeight: 600, fontSize: 14 }}>{post.nftTitle || post.title}</div>}

      {post.nftImages && post.nftImages.length > 1 ? (
        <div style={{ display: 'grid', gridTemplateColumns: 'repeat(auto-fill, minmax(110px, 1fr))', gap: 6, marginTop: 12 }}>
          {post.nftImages.slice(0, 8).map((src, i) => (
            <div key={i} style={{ position: 'relative' }}>
              <img src={src} alt="" style={{ width: '100%', aspectRatio: '1 / 1', objectFit: 'cover', borderRadius: 8, display: 'block' }} />
              {i === 7 && post.nftImages!.length > 8 && (
                <span style={{ position: 'absolute', inset: 0, background: 'rgba(0,0,0,.55)', borderRadius: 8, display: 'flex', alignItems: 'center', justifyContent: 'center', fontWeight: 700 }}>
                  +{post.nftImages!.length - 8}
                </span>
              )}
            </div>
          ))}
        </div>
      ) : (post.nftImage || post.nftImages?.[0]) && (
        <img src={post.nftImage || post.nftImages![0]} alt={post.nftTitle ?? ''}
          style={{ width: '100%', maxHeight: 360, objectFit: 'contain', background: 'var(--bg-soft)', borderRadius: 10, marginTop: 12, display: 'block' }} />
      )}

      {post.forSale && (
        <div style={{ marginTop: 10 }}>
          <span className="badge badge-success">у продажу · {fmtPrice(post.price, post.currency)}</span>
        </div>
      )}

      <div style={{ display: 'flex', gap: 12, marginTop: 14, alignItems: 'center' }}>
        <button
          className="btn"
          onClick={onLike}
          style={liked ? { color: 'var(--primary)', borderColor: 'var(--primary)' } : undefined}
        >
          {liked ? '❤️' : '🤍'} {post.likes ?? 0}
        </button>
        <span className="sub">{post.comments?.length ?? 0} {plural(post.comments?.length ?? 0, 'коментар', 'коментарі', 'коментарів')}</span>
      </div>

      {post.comments && post.comments.length > 0 && (
        <div style={{ marginTop: 14, borderTop: '1px solid var(--border-soft)', paddingTop: 10 }}>
          {post.comments.slice(-3).map(c => (
            <div key={c.id} style={{ fontSize: 13, padding: '4px 0' }}>{c.text}</div>
          ))}
        </div>
      )}

      <div style={{ display: 'flex', gap: 8, marginTop: 10 }}>
        <input
          placeholder="Написати коментар…"
          value={commentText}
          onChange={e => setCommentText(e.target.value)}
          style={{ flex: 1, background: 'var(--bg-soft)', border: '1px solid var(--border)', borderRadius: 10, padding: '8px 12px', color: 'var(--text)', fontSize: 13 }}
        />
        <button
          className="btn btn-primary"
          disabled={!commentText.trim()}
          onClick={() => { onComment(commentText); setCommentText(''); }}
        >
          Надіслати
        </button>
      </div>
    </div>
  );
}
