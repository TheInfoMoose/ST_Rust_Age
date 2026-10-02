import re

with open("simply-transfer-core/src/engine.rs", "r") as f:
    content = f.read()

replacement = """
                    let normalized_file = f.replace('\\\\', "/");
                    let full_remote_path = format!("{}/{}", dest_dir.trim_end_matches(&['/', '\\\\'][..]), normalized_file);
"""

pattern = re.compile(
    r'let normalized_file = f\.replace\(\'\\\\\'\, "\/"\);\s*let full_remote_path = if active_source_dir_val\.is_file\(\) \{.*?\} else \{.*?\};\s*',
    re.DOTALL
)

content = pattern.sub(replacement.lstrip(), content, count=1)

with open("simply-transfer-core/src/engine.rs", "w") as f:
    f.write(content)

