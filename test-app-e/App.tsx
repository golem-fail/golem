import { StatusBar } from 'expo-status-bar';
import { useState } from 'react';
import { Pressable, StyleSheet, Text, View } from 'react-native';

// Minimal counter used by golem's e2e to verify the Expo install script
// builds, installs, launches, and drives. Mirrors test-app / test-app-b:
// a "Counter" heading, a count value below it, and Increment/Decrement
// buttons labelled for both text (`+`/`-`) and accessibility selectors.
export default function App() {
  const [count, setCount] = useState(0);
  return (
    <View style={styles.container}>
      <Text style={styles.heading}>Counter</Text>
      {/* No accessibilityLabel: the visible number IS the accessible label,
          so golem can match/assert on it by text (e.g. on_text = "1"). */}
      <Text style={styles.count} testID="count">
        {count}
      </Text>
      <View style={styles.row}>
        {/* accessibilityLabel is the glyph itself so the button is matchable
            by on_text = "-"/"+". Overriding it with a word (e.g. "Decrement")
            would collapse the child Text and hide the glyph from the tree. */}
        <Pressable
          accessibilityLabel="-"
          testID="decrement"
          style={styles.button}
          onPress={() => setCount((c) => c - 1)}
        >
          <Text style={styles.buttonText}>-</Text>
        </Pressable>
        <Pressable
          accessibilityLabel="+"
          testID="increment"
          style={styles.button}
          onPress={() => setCount((c) => c + 1)}
        >
          <Text style={styles.buttonText}>+</Text>
        </Pressable>
      </View>
      <StatusBar style="auto" />
    </View>
  );
}

const styles = StyleSheet.create({
  container: {
    flex: 1,
    backgroundColor: '#fff',
    alignItems: 'center',
    justifyContent: 'center',
    gap: 16,
  },
  heading: { fontSize: 24, fontWeight: '600' },
  count: { fontSize: 48, fontWeight: '700' },
  row: { flexDirection: 'row', gap: 24 },
  button: {
    width: 64,
    height: 64,
    borderRadius: 12,
    backgroundColor: '#2563eb',
    alignItems: 'center',
    justifyContent: 'center',
  },
  buttonText: { color: '#fff', fontSize: 32, fontWeight: '700' },
});
