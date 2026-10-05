import { Buffer } from 'buffer';
import process from 'process';

if (typeof window !== 'undefined') {
  (window as any).global = (window as any).global || window;
  (window as any).Buffer = (window as any).Buffer || Buffer;
  (window as any).process = process;
}
