package fail.golem.companion;

import android.app.UiAutomation;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import org.junit.Test;
import org.junit.runner.RunWith;
import org.json.JSONObject;

import java.io.OutputStream;
import java.io.InputStream;
import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.net.HttpURLConnection;
import java.net.URL;
import java.nio.charset.StandardCharsets;

@RunWith(AndroidJUnit4.class)
public class CompanionServerTest {
    @Test
    public void startServer() throws Exception {
        UiAutomation uiAutomation = InstrumentationRegistry.getInstrumentation().getUiAutomation();
        android.os.Bundle args = InstrumentationRegistry.getArguments();

        String deviceSerial = args.getString("device_serial", "unknown");

        // Read registration port from args (golem passes this)
        int regPort = 0;
        String regPortStr = args.getString("reg_port");
        if (regPortStr != null) {
            try { regPort = Integer.parseInt(regPortStr); } catch (NumberFormatException ignored) {}
        }

        int port;
        if (regPort > 0) {
            // Register with golem to get our port allocation
            port = registerWithGolem(regPort, deviceSerial);
        } else {
            // Fallback: use port from args or default (for manual/legacy startup)
            port = 8223;
            String portStr = args.getString("port");
            if (portStr != null) {
                try { port = Integer.parseInt(portStr); } catch (NumberFormatException ignored) {}
            }
        }

        CompanionServer server = new CompanionServer(uiAutomation, port, deviceSerial);

        // If using registration, set up re-registration callback for port conflicts
        if (regPort > 0) {
            final int rp = regPort;
            final String ds = deviceSerial;
            server.setPortAllocator(() -> registerWithGolem(rp, ds));
        }

        server.start();
    }

    /**
     * Register with golem's registration server to get a port allocation.
     * The registration server is reachable via ADB reverse on the given port.
     */
    private int registerWithGolem(int regPort, String deviceSerial) throws Exception {
        JSONObject body = new JSONObject();
        body.put("platform", "android");
        body.put("device_id", deviceSerial);
        body.put("device_name", android.os.Build.MODEL);
        body.put("version", "0.4.3");

        URL url = new URL("http://localhost:" + regPort + "/register");
        HttpURLConnection conn = (HttpURLConnection) url.openConnection();
        conn.setRequestMethod("POST");
        conn.setRequestProperty("Content-Type", "application/json");
        conn.setDoOutput(true);
        conn.setConnectTimeout(5000);
        conn.setReadTimeout(5000);

        try (OutputStream os = conn.getOutputStream()) {
            os.write(body.toString().getBytes(StandardCharsets.UTF_8));
        }

        try (BufferedReader reader = new BufferedReader(
                new InputStreamReader(conn.getInputStream(), StandardCharsets.UTF_8))) {
            StringBuilder response = new StringBuilder();
            String line;
            while ((line = reader.readLine()) != null) {
                response.append(line);
            }
            JSONObject resp = new JSONObject(response.toString());
            return resp.getInt("port");
        }
    }
}
