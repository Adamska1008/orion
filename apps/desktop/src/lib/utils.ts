import { clsx, type ClassValue } from 'clsx';
import { twMerge } from 'tailwind-merge';
export function cn(...inputs: ClassValue[]) { return twMerge(clsx(inputs)); }

export function formatBytes(value: number | null): string {
  if (value === null) return '未知';
  if (value === 0) return '0 B';
  const unit = Math.min(Math.floor(Math.log(value) / Math.log(1024)), 5);
  return `${(value / 1024 ** unit).toLocaleString('zh-CN', { maximumFractionDigits: unit ? 1 : 0 })} ${['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB'][unit]}`;
}

export function displayPath(path: string): string {
  if (path.startsWith('\\\\?\\UNC\\')) return '\\\\' + path.slice(8);
  return path.replace(/^\\\\\?\\/, '');
}

export function formatDate(value: number | null): string {
  return value === null ? '未知' : new Date(value).toLocaleString('zh-CN', { hour12: false });
}
