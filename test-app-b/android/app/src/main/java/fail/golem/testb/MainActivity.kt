package fail.golem.testb

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.compose.foundation.layout.*
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material3.*
import androidx.compose.runtime.*
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTag
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import androidx.core.content.ContextCompat
import kotlinx.coroutines.flow.MutableStateFlow

/// Process-lifetime notification store updated by the broadcast
/// receiver below. TestScreen observes it for the
/// `notification-display-b` element that push_notification.test
/// asserts against.
object NotificationStore {
    val latestBody = MutableStateFlow("")
}

class MainActivity : ComponentActivity() {
    /// Receives the broadcast that `golem-driver` sends via
    /// `adb shell am broadcast -a fail.golem.testb.PUSH_NOTIFICATION
    /// --es body "..."`. Pulls the body extra into the store; the
    /// Compose UI re-renders. Mirrors test-app-b's iOS path where
    /// UNUserNotificationCenterDelegate updates `NotificationStore`.
    private val receiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context?, intent: Intent?) {
            val body = intent?.getStringExtra("body") ?: ""
            NotificationStore.latestBody.value = body
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        // RECEIVER_EXPORTED is required for `adb shell am broadcast`
        // to reach us on Android 13+ — the adb process is external to
        // the app, so an internal-only receiver would silently drop
        // the broadcast. The intent action is namespaced under the
        // bundle id so other apps' receivers don't intercept it.
        ContextCompat.registerReceiver(
            this,
            receiver,
            IntentFilter("fail.golem.testb.PUSH_NOTIFICATION"),
            ContextCompat.RECEIVER_EXPORTED,
        )
        setContent {
            MaterialTheme {
                Surface(modifier = Modifier.fillMaxSize()) {
                    TestScreen()
                }
            }
        }
    }

    override fun onDestroy() {
        super.onDestroy()
        unregisterReceiver(receiver)
    }
}

@Composable
fun TestScreen() {
    var counter by remember { mutableStateOf(0) }
    var status by remember { mutableStateOf("Ready") }
    var toggleOn by remember { mutableStateOf(false) }
    val notification by NotificationStore.latestBody.collectAsState()

    Column(
        modifier = Modifier
            .fillMaxSize()
            .padding(24.dp),
        horizontalAlignment = Alignment.CenterHorizontally,
        verticalArrangement = Arrangement.spacedBy(16.dp)
    ) {
        Text(
            "GOLEM Test B",
            fontSize = 28.sp,
            modifier = Modifier.semantics { contentDescription = "app-b-title" }
        )

        // Updated by the BroadcastReceiver above on every
        // push_notification broadcast. push_notification.test asserts
        // the body text shows up here.
        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            Text("Notification:")
            Text(
                notification,
                modifier = Modifier.semantics {
                    contentDescription = "notification-display-b"
                }
            )
        }

        Text(
            status,
            modifier = Modifier.semantics { contentDescription = "status-label" }
        )

        Text(
            "Shared Data",
            modifier = Modifier.semantics { contentDescription = "shared-data-display" }
        )

        Button(
            onClick = { status = "Refreshed" },
            modifier = Modifier.semantics { contentDescription = "refresh-button" }
        ) {
            Text("Refresh")
        }

        Divider()

        // Counter for accessibility_id testing
        Text(
            "$counter",
            fontSize = 24.sp,
            modifier = Modifier.semantics { contentDescription = "counter-b" }
        )

        Row(horizontalArrangement = Arrangement.spacedBy(16.dp)) {
            Button(
                onClick = { counter++ },
                modifier = Modifier.semantics { contentDescription = "increment-b" }
            ) {
                Text("Increment")
            }

            Button(
                onClick = { counter-- },
                modifier = Modifier.semantics { contentDescription = "decrement-b" }
            ) {
                Text("Decrement")
            }
        }

        Row(
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.spacedBy(8.dp)
        ) {
            Text("Test Toggle")
            Switch(
                checked = toggleOn,
                onCheckedChange = { toggleOn = it },
                modifier = Modifier.semantics { contentDescription = "toggle-b" }
            )
        }

        Divider()

        Text(
            "Native Scroll List",
            fontSize = 18.sp,
            modifier = Modifier.semantics { contentDescription = "native-list-title" }
        )

        // Native LazyColumn in a fixed-height container — items beyond 200dp are clipped
        LazyColumn(
            modifier = Modifier
                .height(200.dp)
                .fillMaxWidth()
                .semantics { contentDescription = "native-list" }
        ) {
            items((0..49).toList()) { index ->
                Text(
                    "Native Item $index",
                    modifier = Modifier
                        .fillMaxWidth()
                        .padding(horizontal = 16.dp, vertical = 8.dp)
                        .semantics { contentDescription = "native-item-$index" }
                )
            }
        }
    }
}
