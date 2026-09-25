package fail.golem.companion;

import static org.junit.Assert.assertEquals;
import static org.junit.Assert.assertFalse;
import static org.junit.Assert.assertTrue;

import android.app.UiAutomation;

import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;

import org.json.JSONObject;
import org.junit.After;
import org.junit.Before;
import org.junit.Test;
import org.junit.runner.RunWith;

import java.io.BufferedReader;
import java.io.IOException;
import java.io.InputStream;
import java.io.InputStreamReader;
import java.io.OutputStream;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;

/**
 * Bounded, host-free coverage of the companion's HTTP surface.
 *
 * <p>Distinct from {@link CompanionServerTest}, which is the runtime entry
 * point: it binds the host-allocated port and blocks in {@code start()}
 * forever. This binds an ephemeral port, drives it over loopback, and stops
 * it — so it can run under {@code connectedCheck} with no golem host present.
 *
 * <p>Every request carries a read timeout: the bugs worth catching here (a
 * char-based body read, a response never written) manifest as a hang, not as
 * a wrong answer.
 */
@RunWith(AndroidJUnit4.class)
public class CompanionSmokeTest {

    /** Generous next to a loopback round trip, far under the CI job timeout. */
    private static final int REQUEST_TIMEOUT_MS = 10_000;
    private static final long BIND_TIMEOUT_MS = 10_000L;
    private static final String SERIAL = "smoke-serial";

    private CompanionServer server;
    private Thread serverThread;
    private int port;

    @Before
    public void startServerOnAnEphemeralPort() throws Exception {
        UiAutomation uiAutomation =
                InstrumentationRegistry.getInstrumentation().getUiAutomation();
        // Port 0 → the OS picks a free one, so the suite never collides with a
        // real companion (8223) or with a leftover socket from a prior run.
        server = new CompanionServer(uiAutomation, 0, SERIAL);
        serverThread = new Thread(() -> {
            try {
                server.start();
            } catch (IOException e) {
                throw new RuntimeException(e);
            }
        });
        serverThread.start();

        long deadline = System.currentTimeMillis() + BIND_TIMEOUT_MS;
        while (server.boundPort() <= 0) {
            if (System.currentTimeMillis() > deadline) {
                throw new AssertionError("server never bound a port");
            }
            Thread.sleep(20);
        }
        port = server.boundPort();
    }

    @After
    public void stopServer() throws Exception {
        if (server != null) server.stop();
        if (serverThread != null) serverThread.join(5_000);
    }

    @Test
    public void anUnknownPathReturns404() throws Exception {
        Response r = request("GET", "/no-such-endpoint", null);
        assertEquals(404, r.code);
        assertEquals("not found", new JSONObject(r.body).getString("error"));
    }

    @Test
    public void aMultibyteUtf8BodyDoesNotStallTheRequest() throws Exception {
        // 300 chars, 900 UTF-8 bytes. A char-counting body read waits for 900
        // chars, gets 300, and blocks until the client's read timeout — so a
        // response arriving at all is the assertion.
        StringBuilder sb = new StringBuilder();
        for (int i = 0; i < 300; i++) sb.append("日");
        String body = new JSONObject().put("text", sb.toString()).toString();
        assertTrue(body.getBytes(StandardCharsets.UTF_8).length > body.length());

        Response r = request("POST", "/no-such-endpoint", body);
        assertEquals(404, r.code);
    }

    @Test
    public void healthReportsTheDeviceIdentityItWasConstructedWith() throws Exception {
        Response r = request("GET", "/health", null);
        // 200 once the accessibility binding is warm, 503 while it is not.
        // Which one depends on emulator timing; both carry the identity block,
        // and asserting on it keeps this independent of that race.
        assertTrue("unexpected status " + r.code, r.code == 200 || r.code == 503);

        JSONObject json = new JSONObject(r.body);
        assertEquals("android", json.getString("platform"));
        assertEquals(SERIAL, json.getString("device_id"));
        assertFalse(json.getString("version").isEmpty());
        assertTrue(json.has("max_recording_width"));
    }

    @Test
    public void stoppingTheServerReturnsFromStart() throws Exception {
        server.stop();
        serverThread.join(5_000);
        assertFalse("start() did not return after stop()", serverThread.isAlive());
        // Hand nothing back to @After: a second stop() happens to be harmless,
        // but this test should not be the thing that proves it.
        server = null;
        serverThread = null;
    }

    private static final class Response {
        final int code;
        final String body;

        Response(int code, String body) {
            this.code = code;
            this.body = body;
        }
    }

    private Response request(String method, String path, String body) throws Exception {
        HttpURLConnection conn =
                (HttpURLConnection) new URL("http://127.0.0.1:" + port + path).openConnection();
        conn.setRequestMethod(method);
        conn.setConnectTimeout(REQUEST_TIMEOUT_MS);
        conn.setReadTimeout(REQUEST_TIMEOUT_MS);
        if (body != null) {
            conn.setRequestProperty("Content-Type", "application/json");
            conn.setDoOutput(true);
            try (OutputStream os = conn.getOutputStream()) {
                os.write(body.getBytes(StandardCharsets.UTF_8));
            }
        }
        int code = conn.getResponseCode();
        InputStream in = code >= 400 ? conn.getErrorStream() : conn.getInputStream();
        StringBuilder sb = new StringBuilder();
        if (in != null) {
            try (BufferedReader reader =
                         new BufferedReader(new InputStreamReader(in, StandardCharsets.UTF_8))) {
                String line;
                while ((line = reader.readLine()) != null) sb.append(line);
            }
        }
        return new Response(code, sb.toString());
    }
}
