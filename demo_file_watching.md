# File Watching Demo

## How to Test File Watching in TimeLoop Terminal

1. **Start a new session:**
   ```bash
   cargo run -- start --name "file-watch-test"
   ```

2. **In the terminal session, you should see:**
   ```
   🚀 TimeLoop Terminal - Your terminal is a time machine!
   Type 'exit' or press Ctrl+C to quit
   ────────────────────────────────────────────────────────────
   📁 File watching started for: /path/to/your/project
   🕰️  /path/to/your/project $
   ```

3. **Create some test files:**
   ```bash
   echo "Hello, TimeLoop!" > test1.txt
   echo "Another test file" > test2.txt
   ```

4. **Modify a file:**
   ```bash
   echo "Modified content" >> test1.txt
   ```

5. **Exit the session:**
   ```bash
   exit
   ```

6. **Check the session summary:**
   ```bash
   cargo run -- list
   cargo run -- summary <session-id>
   ```

7. **Replay the session to see file changes:**
   ```bash
   cargo run -- replay <session-id>
   ```

## Expected Results

- The session summary should show files modified > 0
- The replay should show file creation and modification events
- File changes should be recorded with timestamps and file paths 