# Examples

These files exist solely to give the CLI something synthetic to chew on during development. They are **not** a real wordlist or a real corpus of hashes — do not use them for any real audit.

## Files

- `tiny_dict.txt` — 10 common-knowledge passwords (used as candidates).
- `sample_hashes.txt` — MD5 digests of the lines in `tiny_dict.txt`, one per line, in the same order.

## Regenerating

If you change `tiny_dict.txt`, regenerate `sample_hashes.txt`. PowerShell:

```powershell
Get-Content examples\tiny_dict.txt | ForEach-Object {
    $bytes = [Text.Encoding]::UTF8.GetBytes($_)
    $md5   = [Security.Cryptography.MD5]::Create()
    ($md5.ComputeHash($bytes) | ForEach-Object { $_.ToString('x2') }) -join ''
} | Set-Content -Encoding ascii examples\sample_hashes.txt
```
