package com.shunet.android;

import java.io.IOException;
import java.math.BigInteger;
import java.util.Arrays;

/** Implements the portal's RSAUtils format, not PKCS#1 padding. */
final class PortalRsa {
    private PortalRsa() {}

    static String encrypt(String text, String exponentHex, String modulusHex) throws IOException {
        try {
            String hex = modulusHex.trim().replaceFirst("(?i)^0x", "");
            if (!hex.matches("[0-9a-fA-F]{8,2048}")) throw new IllegalArgumentException();
            BigInteger modulus = new BigInteger(hex, 16);
            BigInteger exponent = new BigInteger(exponentHex.trim().replaceFirst("(?i)^0x", ""), 16);
            if (exponent.signum() <= 0 || exponent.bitLength() > 32) throw new IllegalArgumentException();
            int chunkSize = 2 * ((modulus.bitLength() + 15) / 16 - 1);
            if (chunkSize <= 0) throw new IllegalArgumentException();
            char[] chars = text.toCharArray();
            // Reverse UTF-16 code units exactly as the supplied portal JavaScript.
            for (int i = 0, j = chars.length - 1; i < j; i++, j--) {
                char value = chars[i]; chars[i] = chars[j]; chars[j] = value;
            }
            StringBuilder output = new StringBuilder();
            try {
                for (int start = 0; start < chars.length; start += chunkSize) {
                    byte[] bigEndian = new byte[chunkSize];
                    for (int i = 0; i < chunkSize && start + i < chars.length; i++) {
                        bigEndian[chunkSize - i - 1] = (byte) (chars[start + i] & 255);
                    }
                    BigInteger block = new BigInteger(1, bigEndian);
                    Arrays.fill(bigEndian, (byte) 0);
                    if (block.compareTo(modulus) >= 0) throw new IllegalArgumentException();
                    String encrypted = block.modPow(exponent, modulus).toString(16);
                    if (output.length() > 0) output.append(' ');
                    int padding = (4 - encrypted.length() % 4) % 4;
                    for (int i = 0; i < padding; i++) output.append('0');
                    output.append(encrypted);
                }
            } finally { Arrays.fill(chars, '\0'); }
            return output.toString();
        } catch (RuntimeException ignored) {
            throw new IOException("门户 RSA 参数无效，尚未提交账号密码，请重新查询。");
        }
    }
}
