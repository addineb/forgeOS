# sync-vault.ps1 - refresh the Obsidian vault (a VIEW) from the repo (the
# git-backed source of truth). Run after notes change. Repo stays canonical.
$repo  = "C:\Users\User\.kiro\forgeOS"
$vault = "C:\Users\User\Desktop\obsidian\forgeos"
if (-not (Test-Path $vault)) { New-Item -ItemType Directory -Force -Path $vault | Out-Null }
Copy-Item "$repo\ForgeOS-HOME.md"    $vault -Force
Copy-Item "$repo\ForgeOS-map.canvas" $vault -Force
robocopy "$repo\docs" "$vault\docs" *.md /E /NFL /NDL /NJH /NJS /NP | Out-Null
New-Item -ItemType Directory -Force -Path "$vault\context" | Out-Null
Copy-Item "$repo\.kiro\steering\*.md" "$vault\context\" -Force
Write-Host "vault synced from repo: $vault"