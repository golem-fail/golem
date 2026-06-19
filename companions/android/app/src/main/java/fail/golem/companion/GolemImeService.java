package fail.golem.companion;

import android.content.BroadcastReceiver;
import android.content.Context;
import android.content.Intent;
import android.content.IntentFilter;
import android.inputmethodservice.InputMethodService;
import android.os.Build;
import android.util.Base64;
import android.util.Log;
import android.view.View;
import android.view.inputmethod.InputConnection;

import java.nio.charset.StandardCharsets;

/**
 * Headless IME used only by golem to commit arbitrary Unicode text into
 * the focused field — the path `adb shell input text` can't take
 * (ASCII-only). The host activates this IME (`ime enable` + `ime set`)
 * for the session, then commits text by broadcasting to the receiver
 * registered below; the service writes it through the standard
 * {@link InputConnection#commitText}, so native EditTexts and WebView
 * inputs both receive it.
 *
 * No input view: {@link #onCreateInputView} returns null and the view
 * is never shown, so the keyboard is invisible — it never covers the
 * screen or animates. Text arrives via broadcast, not taps.
 */
public class GolemImeService extends InputMethodService {

    private static final String TAG = "GolemIme";
    /** Broadcast action carrying text to commit. Package-targeted by
     *  the host (`am broadcast -p fail.golem.companion`). */
    static final String ACTION_INPUT = "fail.golem.companion.INPUT_TEXT";
    /** String extra: base64(UTF-8 bytes) of the text to commit. Base64
     *  sidesteps shell quoting and any argv charset ambiguity in
     *  `am broadcast`. A plaintext `msg` extra is accepted as a
     *  fallback for manual testing. */
    static final String EXTRA_B64 = "msg_b64";
    static final String EXTRA_PLAIN = "msg";

    private final BroadcastReceiver inputReceiver = new BroadcastReceiver() {
        @Override
        public void onReceive(Context context, Intent intent) {
            String text = decode(intent);
            if (text == null) {
                setResult(1, "no text extra");
                return;
            }
            InputConnection ic = getCurrentInputConnection();
            if (ic == null) {
                // No field focused under this IME yet — the host
                // re-taps and retries. Report so `am broadcast -w`
                // surfaces it instead of silently dropping the text.
                Log.w(TAG, "commit skipped: no input connection");
                setResult(2, "no input connection");
                return;
            }
            ic.commitText(text, 1);
            setResult(0, "ok");
        }

        private String decode(Intent intent) {
            String b64 = intent.getStringExtra(EXTRA_B64);
            if (b64 != null) {
                return new String(Base64.decode(b64, Base64.DEFAULT), StandardCharsets.UTF_8);
            }
            return intent.getStringExtra(EXTRA_PLAIN);
        }

        private void setResult(int code, String data) {
            if (isOrderedBroadcast()) {
                setResultCode(code);
                setResultData(data);
            }
        }
    };

    @Override
    public void onCreate() {
        super.onCreate();
        IntentFilter filter = new IntentFilter(ACTION_INPUT);
        // API 33+ requires an explicit export flag for runtime receivers.
        // The host (adb shell) is an external sender, so it must be exported.
        if (Build.VERSION.SDK_INT >= 33) {
            registerReceiver(inputReceiver, filter, Context.RECEIVER_EXPORTED);
        } else {
            registerReceiver(inputReceiver, filter);
        }
        Log.i(TAG, "GolemImeService created; listening for " + ACTION_INPUT);
    }

    @Override
    public void onDestroy() {
        try {
            unregisterReceiver(inputReceiver);
        } catch (IllegalArgumentException ignored) {
            // Not registered (onCreate failed) — nothing to undo.
        }
        super.onDestroy();
    }

    /** Invisible keyboard: no input view to inflate. */
    @Override
    public View onCreateInputView() {
        return null;
    }

    /** Never show an input area — text comes from broadcasts. */
    @Override
    public boolean onEvaluateInputViewShown() {
        return false;
    }

    @Override
    public boolean onEvaluateFullscreenMode() {
        return false;
    }
}
