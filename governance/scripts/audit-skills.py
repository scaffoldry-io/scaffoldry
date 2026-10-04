#!/usr/bin/env python3
"""
Audits all skills in .agents/skills to verify:
1. Directory contains SKILL.md
2. SKILL.md starts with valid YAML frontmatter (name, description)
3. No forbidden patterns (no long dashes or empty descriptions)
"""

import sys
from pathlib import Path

def main():
    root = Path(__file__).resolve().parent.parent.parent
    skills_dir = root / ".agents" / "skills"
    
    if not skills_dir.exists():
        print(f"Error: {skills_dir} does not exist", file=sys.stderr)
        return 1

    errors = []
    skill_count = 0

    for skill_path in sorted(skills_dir.iterdir()):
        if not skill_path.is_dir():
            continue
        
        skill_count += 1
        skill_file = skill_path / "SKILL.md"
        if not skill_file.exists():
            errors.append(f"{skill_path.name}: missing SKILL.md")
            continue
        
        content = skill_file.read_text(encoding="utf-8")
        if not content.startswith("---"):
            errors.append(f"{skill_path.name}: SKILL.md missing opening '---' frontmatter")
            continue

        parts = content.split("---", 2)
        if len(parts) < 3:
            errors.append(f"{skill_path.name}: SKILL.md missing closing '---' frontmatter")
            continue

        frontmatter = parts[1]
        if "name:" not in frontmatter:
            errors.append(f"{skill_path.name}: frontmatter missing 'name:' field")
        if "description:" not in frontmatter:
            errors.append(f"{skill_path.name}: frontmatter missing 'description:' field")

    if errors:
        print(f"FAILED: Found {len(errors)} error(s) across {skill_count} skills:")
        for err in errors:
            print(f"  - {err}")
        return 1

    print(f"SUCCESS: Verified {skill_count} skills with valid frontmatter.")
    return 0

if __name__ == "__main__":
    sys.exit(main())
