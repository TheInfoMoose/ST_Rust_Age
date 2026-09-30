# Future Issue: Improve PowerShell Privilege Escalation Prompt

## Description
Currently, when the application needs to run an elevated PowerShell script (e.g., to append keys to `administrators_authorized_keys` on Windows), it uses `Start-Process powershell -Verb RunAs`. This works correctly but causes a visible PowerShell window and a default UAC prompt to pop up, which is not the smoothest or cleanest user experience.

## Goal
Clean up the way we prompt for user confirmation and escalate privileges. 
A good starting point would be to mask or hide the PowerShell window so that only the UAC prompt is visible, or to explore cleaner, native ways to handle privilege escalation on Windows without launching external console windows.

## Notes
- For now, the current implementation is acceptable as it successfully performs the required actions.
- This is a backlog item for a later date to improve overall polish and UX.
