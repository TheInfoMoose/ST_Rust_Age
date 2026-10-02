import os
import glob
import json
from datetime import datetime
import subprocess

def get_transcripts():
    base_dir = os.path.expanduser("~/.gemini/antigravity-ide/brain/")
    # Find all transcript.jsonl files
    pattern = os.path.join(base_dir, "*", ".system_generated", "logs", "transcript.jsonl")
    return glob.glob(pattern)

def parse_timestamps():
    files = get_transcripts()
    timestamps = []
    
    for f in files:
        with open(f, 'r', encoding='utf-8') as fp:
            for line in fp:
                try:
                    data = json.loads(line)
                    # We track active IDE interactions
                    if data.get('created_at'):
                        # Parse ISO format, replacing Z with +00:00 for Python 3.10- compatibility
                        dt = datetime.fromisoformat(data['created_at'].replace('Z', '+00:00'))
                        timestamps.append(dt)
                except Exception:
                    continue
    return sorted(timestamps)

def calculate_time(timestamps, idle_threshold_minutes=20):
    if not timestamps:
        return 0, 0
    
    active_seconds = 0
    session_start = timestamps[0]
    total_span_seconds = 0
    last_t = timestamps[0]
    
    for t in timestamps[1:]:
        delta = (t - last_t).total_seconds()
        
        if delta > idle_threshold_minutes * 60:
            # Idle threshold exceeded. End of session.
            total_span_seconds += (last_t - session_start).total_seconds()
            session_start = t
        else:
            active_seconds += delta
            
        last_t = t
        
    total_span_seconds += (last_t - session_start).total_seconds()
    
    return active_seconds, total_span_seconds

def get_git_commits():
    try:
        # Get count of commits
        result = subprocess.run(
            ['git', 'log', '--oneline'], 
            capture_output=True, text=True, check=True
        )
        # Filter out empty lines
        commits = [line for line in result.stdout.strip().split('\n') if line]
        return len(commits)
    except subprocess.CalledProcessError:
        return 0

def main():
    print("="*60)
    print(" Simply Transfer v2 | Automated Time Tracking Analyzer ")
    print("="*60)
    
    print("Scanning Antigravity IDE Sessions...")
    timestamps = parse_timestamps()
    
    if not timestamps:
        print("No IDE history found in ~/.gemini/antigravity-ide/brain/")
        return

    print(f"Parsed {len(timestamps)} discrete IDE interaction events.")
    
    active_secs, span_secs = calculate_time(timestamps, idle_threshold_minutes=20)
    
    active_hours = active_secs / 3600
    span_hours = span_secs / 3600
    commits = get_git_commits()
    
    print("\n--- High-Level Time Estimate ---")
    print(f"Git Commits:                {commits}")
    print(f"Active Hands-On Development: {active_hours:.1f} hours (20-min idle cutoff)")
    print(f"Total Work Session Span:     {span_hours:.1f} hours")
    
    print("\n--- Resource Allocation Breakdown ---")
    print("Note: IDE interaction tracking establishes total elapsed metrics.")
    print("To dynamically calculate Functional Domain shares, tag your git commits ")
    print("with domain prefixes (e.g., 'UI:', 'CORE:', 'CRYPTO:') going forward.")
    print("="*60)

if __name__ == "__main__":
    main()
