/**
 * 字节流 ⇄ base64。
 * 终端数据必须以字节形式穿越 JSON IPC：字符串化会破坏多字节 UTF-8 与二进制控制序列。
 */

/** base64 -> 原始字节。xterm.write(Uint8Array) 内部按 UTF-8 流式解码，天然处理跨包多字节字符。 */
export function b64ToBytes(b64: string): Uint8Array {
  const binary = atob(b64)
  const bytes = new Uint8Array(binary.length)
  for (let i = 0; i < binary.length; i += 1) {
    bytes[i] = binary.charCodeAt(i)
  }
  return bytes
}

/** 原始字节 -> base64（分块拼接，避免超长参数导致栈溢出）。 */
export function bytesToB64(bytes: Uint8Array): string {
  const chunk = 0x8000
  let binary = ''
  for (let i = 0; i < bytes.length; i += chunk) {
    binary += String.fromCharCode(...bytes.subarray(i, i + chunk))
  }
  return btoa(binary)
}
