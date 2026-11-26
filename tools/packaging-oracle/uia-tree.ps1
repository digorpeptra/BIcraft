# Prints the UI Automation tree (control type, name, automation id) of a process's windows, to
# find the controls a Save As step would drive. Read-only.
# usage: uia-tree.ps1 -ProcessId 1234 [-Depth 6] [-Match text]
param([int]$ProcessId, [int]$Depth = 6, [string]$Match = '')
Add-Type -AssemblyName UIAutomationClient, UIAutomationTypes
$A = [System.Windows.Automation.AutomationElement]
$walker = [System.Windows.Automation.TreeWalker]::ControlViewWalker
function Dump($el, $d) {
  if ($d -gt $Depth) { return }
  $c = $el.Current
  $line = ('  ' * $d) + "$($c.ControlType.ProgrammaticName -replace 'ControlType\.','') '$($c.Name)' id=$($c.AutomationId) class=$($c.ClassName)"
  if (-not $Match -or $line -match $Match) { $line }
  $ch = $walker.GetFirstChild($el)
  while ($ch) { Dump $ch ($d + 1); $ch = $walker.GetNextSibling($ch) }
}
$cond = New-Object System.Windows.Automation.PropertyCondition($A::ProcessIdProperty, $ProcessId)
foreach ($w in $A::RootElement.FindAll([System.Windows.Automation.TreeScope]::Children, $cond)) { Dump $w 0 }
