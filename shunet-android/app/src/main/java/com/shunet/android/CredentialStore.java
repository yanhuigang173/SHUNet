package com.shunet.android;

import android.content.Context;
import android.content.SharedPreferences;
import android.security.keystore.KeyGenParameterSpec;
import android.security.keystore.KeyProperties;
import android.util.Base64;

import java.io.IOException;
import java.nio.ByteBuffer;
import java.nio.charset.StandardCharsets;
import java.security.KeyStore;
import java.util.Arrays;
import javax.crypto.Cipher;
import javax.crypto.KeyGenerator;
import javax.crypto.SecretKey;
import javax.crypto.spec.GCMParameterSpec;

/** No plaintext passwords in preferences, backups, logs, or saved instance state. */
final class CredentialStore {
    private static final String ALIAS = "com.shunet.android.credentials.v1";
    private static final byte[] AAD = "SHUNet.credentials.v1".getBytes(StandardCharsets.UTF_8);
    private final SharedPreferences preferences;

    static final class Saved {
        final String username, password, warning;
        final boolean remember;
        final Operator operator;
        Saved(String username, String password, boolean remember, Operator operator, String warning) {
            this.username = username; this.password = password;
            this.remember = remember; this.operator = operator; this.warning = warning;
        }
    }

    CredentialStore(Context context) {
        preferences = context.getSharedPreferences("credentials", Context.MODE_PRIVATE);
    }

    Saved load() {
        String username = preferences.getString("username", "");
        Operator operator = Operator.fromKey(preferences.getString("operator", "campus"));
        if (!preferences.getBoolean("remember", false)) return new Saved(username, "", false, operator, "");
        byte[] plain = null;
        try {
            byte[] data = Base64.decode(preferences.getString("password", ""), Base64.NO_WRAP);
            if (data.length < 29 || data.length > 4096) throw new IOException("Invalid ciphertext");
            Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
            cipher.init(Cipher.DECRYPT_MODE, key(false),
                    new GCMParameterSpec(128, Arrays.copyOfRange(data, 0, 12)));
            cipher.updateAAD(AAD);
            plain = cipher.doFinal(Arrays.copyOfRange(data, 12, data.length));
            return new Saved(username, new String(plain, StandardCharsets.UTF_8), true, operator, "");
        } catch (Exception ignored) {
            preferences.edit().remove("password").putBoolean("remember", false).apply();
            return new Saved(username, "", false, operator, "已保存的密码无法解密，请重新输入并保存。");
        } finally {
            if (plain != null) Arrays.fill(plain, (byte) 0);
        }
    }

    void saveOperator(Operator operator) {
        // Selecting a carrier must not save or discard unsaved account/password edits.
        preferences.edit().putString("operator", operator.key).apply();
    }

    void save(String username, String password, boolean remember, Operator operator) throws IOException {
        if (username.length() > 128 || password.length() > 128) {
            throw new IOException("账号和密码长度不能超过 128 个字符。");
        }
        String encrypted = null;
        byte[] plain = null;
        try {
            if (remember && !password.isEmpty()) {
                Cipher cipher = Cipher.getInstance("AES/GCM/NoPadding");
                cipher.init(Cipher.ENCRYPT_MODE, key(true));
                cipher.updateAAD(AAD);
                plain = password.getBytes(StandardCharsets.UTF_8);
                byte[] ciphertext = cipher.doFinal(plain);
                encrypted = Base64.encodeToString(ByteBuffer.allocate(12 + ciphertext.length)
                        .put(cipher.getIV()).put(ciphertext).array(), Base64.NO_WRAP);
            }
        } catch (Exception ignored) {
            throw new IOException("本机加密保存失败，原设置已保留，请稍后重试。");
        } finally {
            if (plain != null) Arrays.fill(plain, (byte) 0);
        }
        SharedPreferences.Editor edit = preferences.edit().putString("username", username)
                .putBoolean("remember", encrypted != null).putString("operator", operator.key);
        if (encrypted == null) edit.remove("password"); else edit.putString("password", encrypted);
        if (!edit.commit()) throw new IOException("账号设置写入失败，请检查手机存储空间。");
    }

    private static SecretKey key(boolean create) throws Exception {
        KeyStore store = KeyStore.getInstance("AndroidKeyStore");
        store.load(null);
        if (store.containsAlias(ALIAS)) return (SecretKey) store.getKey(ALIAS, null);
        if (!create) throw new IOException("Keystore key missing");
        KeyGenerator generator = KeyGenerator.getInstance(KeyProperties.KEY_ALGORITHM_AES, "AndroidKeyStore");
        generator.init(new KeyGenParameterSpec.Builder(ALIAS,
                KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT)
                .setBlockModes(KeyProperties.BLOCK_MODE_GCM)
                .setEncryptionPaddings(KeyProperties.ENCRYPTION_PADDING_NONE)
                .setRandomizedEncryptionRequired(true).build());
        return generator.generateKey();
    }
}
