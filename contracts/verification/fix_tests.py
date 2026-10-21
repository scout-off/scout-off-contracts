import os
import re

for root, _, files in os.walk('.'):
    for f in files:
        if f.endswith('.rs'):
            path = os.path.join(root, f)
            with open(path, 'r') as file:
                content = file.read()
            
            original_content = content
            
            # Fix Validator { ... }
            content = re.sub(
                r'(credentials:\s*[^,]+,\s*)(registered_at:)',
                r'\1affiliation: soroban_sdk::String::from_str(&env, "Default"),\n            \2',
                content
            )
            content = re.sub(
                r'(credentials:\s*[^,]+,\s*)(registered_at:.*h\.env)',
                r'\1affiliation: soroban_sdk::String::from_str(&h.env, "Default"),\n            \2',
                content
            )

            with open(path, 'w') as file:
                file.write(content)
