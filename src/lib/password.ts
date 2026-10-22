/** 去掉易混字符 (0/O, 1/l/I) 的字母表; 20 位 ≈ 114 bit, 远超离线暴力破解的可行范围 */
const ALPHABET = "abcdefghijkmnpqrstuvwxyzABCDEFGHJKLMNPQRSTUVWXYZ23456789";
const LENGTH = 20;

/** 拒绝采样避免取模偏差; 每 5 位加一个 `-` 便于抄写 */
export function generateStrongPassword(): string {
  const out: string[] = [];
  const limit = 256 - (256 % ALPHABET.length);
  const buf = new Uint8Array(1);
  while (out.length < LENGTH) {
    crypto.getRandomValues(buf);
    if (buf[0] < limit) out.push(ALPHABET[buf[0] % ALPHABET.length]);
  }
  return out.join("").replace(/(.{5})(?=.)/g, "$1-");
}

/** 与后端 backup/crypto.rs::MIN_PASSWORD_CHARS 一致 */
export const MIN_PASSWORD_CHARS = 10;
