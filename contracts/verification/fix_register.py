import os
import re

for root, _, files in os.walk('.'):
    for f in files:
        if f.endswith('.rs'):
            path = os.path.join(root, f)
            with open(path, 'r') as file:
                content = file.read()
            
            # 3 arguments ending in &Vec::new(...)
            content = re.sub(
                r'(register_validator\s*\(\s*[^,]+,\s*[^,]+(?:&String::from_str\([^,]+,\s*"[^"]+"\))?\s*,\s*)(&Vec::new\([^)]*\))',
                r'\g<1>&soroban_sdk::String::from_str(&env, "Default"), \g<2>',
                content
            )
            
            # 2 arguments
            content = re.sub(
                r'(register_validator\s*\(\s*[^,]+,\s*&String::from_str\([^,]+,\s*"[^"]+"\)\s*)\)',
                r'\1, &soroban_sdk::String::from_str(&env, "Default"), &soroban_sdk::Vec::new(&env))',
                content
            )

            # what about `&credentials` instead of `&String::from_str(...)`
            content = re.sub(
                r'(register_validator\s*\(\s*[^,]+,\s*&credentials\s*)\)',
                r'\1, &soroban_sdk::String::from_str(&env, "Default"), &soroban_sdk::Vec::new(&env))',
                content
            )

            with open(path, 'w') as file:
                file.write(content)
