package fail.golem.companion;

import android.app.UiAutomation;
import android.graphics.Bitmap;
import android.os.ParcelFileDescriptor;
import android.view.accessibility.AccessibilityNodeInfo;
import android.view.accessibility.AccessibilityWindowInfo;

import org.json.JSONArray;
import org.json.JSONObject;

import java.io.BufferedReader;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.io.OutputStream;
import java.net.ServerSocket;
import java.net.Socket;
import java.nio.charset.StandardCharsets;
import java.util.HashMap;
import java.util.List;
import java.util.Map;

public class CompanionServer {

    private static final int DEFAULT_PORT = 8223;
    /** Inactivity timeout — server exits after this duration with no requests. */
    private static final long INACTIVITY_TIMEOUT_MS = 5 * 60 * 60 * 1000L; // 5 hours
    private final UiAutomation uiAutomation;
    private final int port;
    /** ADB serial passed from the host (e.g. "emulator-5554"). */
    private final String deviceSerial;
    private volatile long lastRequestTime = System.currentTimeMillis();

    public CompanionServer(UiAutomation uiAutomation) {
        this(uiAutomation, DEFAULT_PORT, null);
    }

    public CompanionServer(UiAutomation uiAutomation, int port, String deviceSerial) {
        this.uiAutomation = uiAutomation;
        this.port = port;
        this.deviceSerial = deviceSerial != null ? deviceSerial : "unknown";
    }

    public void start() throws IOException {
        startInactivityWatchdog();
        ServerSocket serverSocket = new ServerSocket(port);
        while (true) {
            Socket client = serverSocket.accept();
            new Thread(() -> {
                try {
                    lastRequestTime = System.currentTimeMillis();
                    handleClient(client);
                } catch (Exception e) {
                    e.printStackTrace();
                } finally {
                    try { client.close(); } catch (IOException ignored) {}
                }
            }).start();
        }
    }

    private void startInactivityWatchdog() {
        Thread watchdog = new Thread(() -> {
            while (true) {
                try {
                    Thread.sleep(60_000); // check every minute
                } catch (InterruptedException e) {
                    return;
                }
                long idle = System.currentTimeMillis() - lastRequestTime;
                if (idle >= INACTIVITY_TIMEOUT_MS) {
                    android.util.Log.i("GolemCompanion",
                        "Shutting down after " + (idle / 3600000) + " hours of inactivity");
                    System.exit(0);
                }
            }
        });
        watchdog.setDaemon(true);
        watchdog.start();
    }

    private void handleClient(Socket client) throws Exception {
        BufferedReader reader = new BufferedReader(
                new InputStreamReader(client.getInputStream(), StandardCharsets.UTF_8));
        String requestLine = reader.readLine();
        if (requestLine == null) return;

        String[] parts = requestLine.split(" ");
        if (parts.length < 2) return;

        String method = parts[0];
        String path = parts[1].split("\\?")[0];

        // Read headers
        Map<String, String> headers = new HashMap<>();
        String headerLine;
        while ((headerLine = reader.readLine()) != null && !headerLine.isEmpty()) {
            int colon = headerLine.indexOf(':');
            if (colon > 0) {
                headers.put(headerLine.substring(0, colon).trim().toLowerCase(),
                        headerLine.substring(colon + 1).trim());
            }
        }

        // Read body if present
        String body = "";
        if (headers.containsKey("content-length")) {
            int len = Integer.parseInt(headers.get("content-length"));
            char[] buf = new char[len];
            int read = 0;
            while (read < len) {
                int n = reader.read(buf, read, len - read);
                if (n == -1) break;
                read += n;
            }
            body = new String(buf, 0, read);
        }

        OutputStream out = client.getOutputStream();

        try {
            switch (path) {
                case "/health":
                    sendJson(out, 200, new JSONObject()
                        .put("status", "ok")
                        .put("platform", "android")
                        .put("version", "0.3.1")
                        .put("device_name", android.os.Build.MODEL)
                        .put("device_model", android.os.Build.DEVICE)
                        .put("os_version", String.valueOf(android.os.Build.VERSION.SDK_INT))
                        .put("device_id", deviceSerial));
                    break;
                case "/hierarchy":
                    handleHierarchy(out);
                    break;
                case "/tap":
                    handleTap(out, body);
                    break;
                case "/longpress":
                    handleLongPress(out, body);
                    break;
                case "/type":
                    handleType(out, body);
                    break;
                case "/backspace":
                    handleBackspace(out, body);
                    break;
                case "/swipe":
                    handleSwipe(out, body);
                    break;
                case "/screenshot":
                    handleScreenshot(out);
                    break;
                case "/hide-keyboard":
                    handleHideKeyboard(out);
                    break;
                case "/alert":
                    if ("POST".equals(method)) {
                        handleDismissAlert(out, body);
                    } else {
                        handleGetAlert(out);
                    }
                    break;
                case "/launch":
                    handleLaunch(out, body);
                    break;
                case "/stop":
                    handleStop(out, body);
                    break;
                default:
                    sendJson(out, 404, new JSONObject().put("error", "not found"));
                    break;
            }
        } catch (Exception e) {
            sendJson(out, 500, new JSONObject().put("error", e.getMessage()));
        }
    }

    private void handleHierarchy(OutputStream out) throws Exception {
        AccessibilityNodeInfo root = uiAutomation.getRootInActiveWindow();
        if (root == null) {
            sendJson(out, 500, new JSONObject().put("error", "no active window"));
            return;
        }
        try {
            // Detect keyboard height from IME insets.
            // Scan dumpsys window output for: type=ime frame=[0,TOP][W,BOT] visible=true
            int keyboardHeight = 0;
            try {
                String windowDump = executeShellAndRead("dumpsys window");
                for (String line : windowDump.split("\n")) {
                    if (line.contains("type=ime") && line.contains("visible=true") && line.contains("frame=[")) {
                        int frameIdx = line.indexOf("frame=[");
                        String frameStr = line.substring(frameIdx + 7);
                        String[] parts = frameStr.split("[\\[\\],]+");
                        if (parts.length >= 4) {
                            int top = Integer.parseInt(parts[1].trim());
                            int bottom = Integer.parseInt(parts[3].trim());
                            if (bottom > top && top > 0) {
                                keyboardHeight = bottom - top;
                                break;
                            }
                        }
                    }
                }
            } catch (Exception ignored) {
                // Keyboard detection is best-effort
            }
            JSONObject tree = buildNodeJson(root);
            JSONObject response = new JSONObject();
            response.put("tree", tree);
            response.put("keyboard_height", keyboardHeight);
            sendJson(out, 200, response);
        } finally {
            root.recycle();
        }
    }

    private void handleTap(OutputStream out, String body) throws Exception {
        JSONObject req = new JSONObject(body);
        int x = req.getInt("x");
        int y = req.getInt("y");
        executeShell("input tap " + x + " " + y);
        sendJson(out, 200, new JSONObject().put("status", "ok"));
    }

    private void handleLongPress(OutputStream out, String body) throws Exception {
        JSONObject req = new JSONObject(body);
        int x = req.getInt("x");
        int y = req.getInt("y");
        long duration = req.optLong("duration_ms", 1500);
        executeShell("input swipe " + x + " " + y + " " + x + " " + y + " " + duration);
        sendJson(out, 200, new JSONObject().put("status", "ok"));
    }

    private void handleSwipe(OutputStream out, String body) throws Exception {
        JSONObject req = new JSONObject(body);
        int fromX = req.getInt("from_x");
        int fromY = req.getInt("from_y");
        int toX = req.getInt("to_x");
        int toY = req.getInt("to_y");
        long duration = req.optLong("duration_ms", 300);
        executeShell("input swipe " + fromX + " " + fromY + " " + toX + " " + toY + " " + duration);
        sendJson(out, 200, new JSONObject().put("status", "ok"));
    }

    private void handleType(OutputStream out, String body) throws Exception {
        JSONObject req = new JSONObject(body);
        String text = req.getString("text");
        // Split on newlines — type each line separately with Enter between them.
        // Android's `input text` doesn't support newline characters.
        String[] lines = text.split("\n", -1);
        for (int i = 0; i < lines.length; i++) {
            if (!lines[i].isEmpty()) {
                executeShell("input text " + escapeForInputText(lines[i]));
            }
            if (i < lines.length - 1) {
                executeShell("input keyevent KEYCODE_ENTER");
            }
        }
        sendJson(out, 200, new JSONObject().put("status", "ok"));
    }

    private void handleBackspace(OutputStream out, String body) throws Exception {
        JSONObject req = body.isEmpty() ? new JSONObject() : new JSONObject(body);
        int count = req.optInt("count", 1);
        for (int i = 0; i < count; i++) {
            executeShell("input keyevent DEL");
        }
        sendJson(out, 200, new JSONObject().put("status", "ok"));
    }

    private void handleScreenshot(OutputStream out) throws Exception {
        Bitmap bitmap = uiAutomation.takeScreenshot();
        if (bitmap == null) {
            sendJson(out, 500, new JSONObject().put("error", "screenshot failed"));
            return;
        }
        ByteArrayOutputStream baos = new ByteArrayOutputStream();
        bitmap.compress(Bitmap.CompressFormat.PNG, 100, baos);
        bitmap.recycle();
        byte[] png = baos.toByteArray();

        String header = "HTTP/1.1 200 OK\r\n"
                + "Content-Type: image/png\r\n"
                + "Content-Length: " + png.length + "\r\n"
                + "Connection: close\r\n\r\n";
        out.write(header.getBytes(StandardCharsets.UTF_8));
        out.write(png);
        out.flush();
    }

    private void handleHideKeyboard(OutputStream out) throws Exception {
        executeShell("input keyevent KEYCODE_BACK");
        sendJson(out, 200, new JSONObject().put("status", "ok"));
    }

    private void handleGetAlert(OutputStream out) throws Exception {
        List<AccessibilityWindowInfo> windows = uiAutomation.getWindows();
        for (AccessibilityWindowInfo window : windows) {
            if (window.getType() == AccessibilityWindowInfo.TYPE_SYSTEM) {
                AccessibilityNodeInfo root = window.getRoot();
                if (root != null) {
                    try {
                        JSONObject alert = new JSONObject();
                        alert.put("found", true);
                        alert.put("tree", buildNodeJson(root));
                        sendJson(out, 200, alert);
                        return;
                    } finally {
                        root.recycle();
                    }
                }
            }
        }
        sendJson(out, 200, new JSONObject().put("found", false));
    }

    private void handleDismissAlert(OutputStream out, String body) throws Exception {
        JSONObject req = new JSONObject(body.isEmpty() ? "{}" : body);
        String action = req.optString("action", "dismiss");

        List<AccessibilityWindowInfo> windows = uiAutomation.getWindows();
        for (AccessibilityWindowInfo window : windows) {
            if (window.getType() == AccessibilityWindowInfo.TYPE_SYSTEM) {
                AccessibilityNodeInfo root = window.getRoot();
                if (root != null) {
                    try {
                        if (clickButtonByText(root, action)) {
                            sendJson(out, 200, new JSONObject().put("status", "ok"));
                            return;
                        }
                    } finally {
                        root.recycle();
                    }
                }
            }
        }
        sendJson(out, 404, new JSONObject().put("error", "alert button not found"));
    }

    private boolean clickButtonByText(AccessibilityNodeInfo node, String text) {
        if (node == null) return false;
        CharSequence nodeText = node.getText();
        if (nodeText != null && nodeText.toString().equalsIgnoreCase(text)) {
            node.performAction(AccessibilityNodeInfo.ACTION_CLICK);
            return true;
        }
        for (int i = 0; i < node.getChildCount(); i++) {
            AccessibilityNodeInfo child = node.getChild(i);
            if (child != null) {
                try {
                    if (clickButtonByText(child, text)) return true;
                } finally {
                    child.recycle();
                }
            }
        }
        return false;
    }

    private JSONObject buildNodeJson(AccessibilityNodeInfo node) throws Exception {
        JSONObject json = new JSONObject();
        CharSequence cls = node.getClassName();
        json.put("class", cls != null ? cls.toString() : "");
        CharSequence text = node.getText();
        json.put("text", text != null ? text.toString() : "");
        CharSequence desc = node.getContentDescription();
        json.put("contentDescription", desc != null ? desc.toString() : "");
        json.put("clickable", node.isClickable());
        json.put("enabled", node.isEnabled());
        json.put("focused", node.isFocused());
        json.put("scrollable", node.isScrollable());
        json.put("selected", node.isSelected());
        json.put("checked", node.isChecked());

        android.graphics.Rect bounds = new android.graphics.Rect();
        node.getBoundsInScreen(bounds);
        JSONObject boundsJson = new JSONObject();
        boundsJson.put("left", bounds.left);
        boundsJson.put("top", bounds.top);
        boundsJson.put("right", bounds.right);
        boundsJson.put("bottom", bounds.bottom);
        json.put("bounds", boundsJson);

        JSONArray children = new JSONArray();
        for (int i = 0; i < node.getChildCount(); i++) {
            AccessibilityNodeInfo child = node.getChild(i);
            if (child != null) {
                try {
                    children.put(buildNodeJson(child));
                } finally {
                    child.recycle();
                }
            }
        }
        json.put("children", children);
        return json;
    }

    private void executeShell(String command) throws IOException {
        ParcelFileDescriptor pfd = uiAutomation.executeShellCommand(command);
        try (InputStream is = new ParcelFileDescriptor.AutoCloseInputStream(pfd)) {
            byte[] buf = new byte[1024];
            while (is.read(buf) != -1) { /* drain */ }
        }
    }

    private String executeShellAndRead(String command) throws Exception {
        ParcelFileDescriptor pfd = uiAutomation.executeShellCommand(command);
        try (InputStream is = new ParcelFileDescriptor.AutoCloseInputStream(pfd);
             BufferedReader reader = new BufferedReader(new InputStreamReader(is))) {
            StringBuilder sb = new StringBuilder();
            String line;
            while ((line = reader.readLine()) != null) {
                sb.append(line).append("\n");
            }
            return sb.toString();
        }
    }

    private String escapeForInputText(String text) {
        StringBuilder sb = new StringBuilder();
        for (char c : text.toCharArray()) {
            switch (c) {
                case ' ':  sb.append("%s"); break;
                case '&':  sb.append("\\&"); break;
                case '<':  sb.append("\\<"); break;
                case '>':  sb.append("\\>"); break;
                case '|':  sb.append("\\|"); break;
                case ';':  sb.append("\\;"); break;
                case '(':  sb.append("\\("); break;
                case ')':  sb.append("\\)"); break;
                case '$':  sb.append("\\$"); break;
                case '`':  sb.append("\\`"); break;
                case '"':  sb.append("\\\""); break;
                case '\'': sb.append("\\'"); break;
                case '\\': sb.append("\\\\"); break;
                default:   sb.append(c); break;
            }
        }
        return sb.toString();
    }

    private void handleLaunch(OutputStream out, String body) throws Exception {
        JSONObject req = new JSONObject(body);
        String packageName = req.optString("bundle_id", "");
        if (packageName.isEmpty()) {
            sendJson(out, 400, new JSONObject().put("error", "missing bundle_id"));
            return;
        }
        // Use monkey to launch — more reliable than am start via executeShellCommand
        String cmd = "monkey -p " + packageName + " -c android.intent.category.LAUNCHER 1";
        ParcelFileDescriptor pfd = uiAutomation.executeShellCommand(cmd);
        try (InputStream is = new ParcelFileDescriptor.AutoCloseInputStream(pfd)) {
            byte[] buf = new byte[4096];
            while (is.read(buf) != -1) { /* drain */ }
        }
        sendJson(out, 200, new JSONObject().put("status", "ok"));
    }

    private void handleStop(OutputStream out, String body) throws Exception {
        JSONObject req = new JSONObject(body);
        String packageName = req.optString("bundle_id", "");
        if (packageName.isEmpty()) {
            sendJson(out, 400, new JSONObject().put("error", "missing bundle_id"));
            return;
        }
        ParcelFileDescriptor pfd = uiAutomation.executeShellCommand("am force-stop " + packageName);
        try (InputStream is = new ParcelFileDescriptor.AutoCloseInputStream(pfd)) {
            byte[] buf = new byte[4096];
            while (is.read(buf) != -1) { /* drain */ }
        }
        sendJson(out, 200, new JSONObject().put("status", "ok"));
    }

    private void sendJson(OutputStream out, int status, JSONObject json) throws IOException {
        String statusText;
        switch (status) {
            case 200: statusText = "OK"; break;
            case 400: statusText = "Bad Request"; break;
            case 404: statusText = "Not Found"; break;
            case 500: statusText = "Internal Server Error"; break;
            default:  statusText = "Unknown"; break;
        }
        byte[] body = json.toString().getBytes(StandardCharsets.UTF_8);
        String header = "HTTP/1.1 " + status + " " + statusText + "\r\n"
                + "Content-Type: application/json\r\n"
                + "Content-Length: " + body.length + "\r\n"
                + "Connection: close\r\n\r\n";
        out.write(header.getBytes(StandardCharsets.UTF_8));
        out.write(body);
        out.flush();
    }
}
