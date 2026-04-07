package fail.golem.companion;

import android.app.UiAutomation;
import androidx.test.ext.junit.runners.AndroidJUnit4;
import androidx.test.platform.app.InstrumentationRegistry;
import org.junit.Test;
import org.junit.runner.RunWith;

@RunWith(AndroidJUnit4.class)
public class CompanionServerTest {
    @Test
    public void startServer() throws Exception {
        UiAutomation uiAutomation = InstrumentationRegistry.getInstrumentation().getUiAutomation();

        // Read port from instrumentation args, default to 8223
        android.os.Bundle args = InstrumentationRegistry.getArguments();
        int port = 8223;
        String portStr = args.getString("port");
        if (portStr != null) {
            try {
                port = Integer.parseInt(portStr);
            } catch (NumberFormatException ignored) {}
        }

        String deviceSerial = args.getString("device_serial");

        CompanionServer server = new CompanionServer(uiAutomation, port, deviceSerial);
        server.start();
    }
}
