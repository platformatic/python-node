use arwen::elf::ElfContainer;
use arwen::macho::MachoContainer;
use std::{
  collections::HashMap,
  env,
  fs::{self, File},
  path::Path,
};

fn is_elf_binary(file_contents: &[u8]) -> bool {
  file_contents.len() >= 4 && &file_contents[0..4] == b"\x7fELF"
}

fn is_macho_binary(file_contents: &[u8]) -> bool {
  if file_contents.len() < 4 {
    return false;
  }

  let magic = u32::from_ne_bytes([
    file_contents[0],
    file_contents[1],
    file_contents[2],
    file_contents[3],
  ]);

  // Mach-O magic numbers
  magic == 0xfeedface || // 32-bit
  magic == 0xfeedfacf || // 64-bit
  magic == 0xcafebabe || // Fat binary
  magic == 0xcefaedfe || // 32-bit swapped
  magic == 0xcffaedfe // 64-bit swapped
}

fn find_python_library_macos() -> Result<String, String> {
  eprintln!("fix-python-soname: Looking for Python framework on macOS...");

  // Python versions from 3.20 down to 3.8
  let mut python_versions = Vec::new();
  for major in (8..=20).rev() {
    // Framework paths (highest priority)
    python_versions.push(format!("Python.framework/Versions/3.{}/Python", major));
  }

  eprintln!(
    "fix-python-soname: Looking for versions: {:?}",
    &python_versions[0..6]
  );

  // macOS Python search paths (ordered by priority)
  let mut lib_paths = vec![
    // Homebrew paths (most common first)
    "/opt/homebrew/opt/python@3.13/Frameworks",
    "/opt/homebrew/opt/python@3.12/Frameworks",
    "/opt/homebrew/opt/python@3.11/Frameworks",
    "/opt/homebrew/opt/python@3.10/Frameworks",
    "/opt/homebrew/opt/python@3.9/Frameworks",
    "/opt/homebrew/opt/python@3.8/Frameworks",
    // Intel Mac Homebrew
    "/usr/local/opt/python@3.13/Frameworks",
    "/usr/local/opt/python@3.12/Frameworks",
    "/usr/local/opt/python@3.11/Frameworks",
    "/usr/local/opt/python@3.10/Frameworks",
    "/usr/local/opt/python@3.9/Frameworks",
    "/usr/local/opt/python@3.8/Frameworks",
    // System Python frameworks
    "/Library/Frameworks",
    "/System/Library/Frameworks",
  ];

  // Check for active virtual environments first
  if let Ok(venv) = env::var("VIRTUAL_ENV") {
    let venv_fw = format!("{}/Frameworks", venv);
    lib_paths.insert(0, Box::leak(venv_fw.into_boxed_str()));
  }

  // Add user-specific paths
  if let Ok(home) = env::var("HOME") {
    // pyenv installations
    let pyenv_versions = format!("{}/.pyenv/versions", home);
    if let Ok(entries) = fs::read_dir(&pyenv_versions) {
      for entry in entries.flatten() {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
          let version_fw = format!("{}/Frameworks", entry.path().display());
          lib_paths.push(Box::leak(version_fw.into_boxed_str()));
        }
      }
    }
  }

  eprintln!(
    "fix-python-soname: Searching in {} framework directories...",
    lib_paths.len()
  );

  // First try exact version matches
  for lib_name in &python_versions {
    for lib_path in &lib_paths {
      let full_path = format!("{}/{}", lib_path, lib_name);
      if std::path::Path::new(&full_path).exists() {
        eprintln!(
          "fix-python-soname: Found Python framework: {} at {}",
          lib_name, full_path
        );
        return Ok(full_path);
      }
    }
  }

  eprintln!("fix-python-soname: No exact match found, searching for any Python.framework...");

  // If no exact match found, search directories for any Python frameworks
  for lib_path in &lib_paths {
    if let Ok(entries) = fs::read_dir(lib_path) {
      let mut found_frameworks: Vec<(String, u32, u32)> = Vec::new();

      for entry in entries.flatten() {
        if let Some(name) = entry.file_name().to_str() {
          if name == "Python.framework" {
            // Check for version directories
            let versions_dir = entry.path().join("Versions");
            if let Ok(version_entries) = fs::read_dir(&versions_dir) {
              for version_entry in version_entries.flatten() {
                if let Some(version_name) = version_entry.file_name().to_str() {
                  if let Some(version_start) = version_name.find("3.") {
                    let version_part = &version_name[version_start + 2..];
                    if let Ok(minor) = version_part.parse::<u32>() {
                      let python_path = version_entry.path().join("Python");
                      if python_path.exists() {
                        found_frameworks.push((
                          python_path.to_string_lossy().to_string(),
                          3,
                          minor,
                        ));
                      }
                    }
                  }
                }
              }
            }
          }
        }
      }

      // Sort by version (newest first)
      found_frameworks.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)));

      if let Some((framework_path, _, _)) = found_frameworks.first() {
        eprintln!(
          "fix-python-soname: Found Python framework: {} in {}",
          framework_path, lib_path
        );
        return Ok(framework_path.clone());
      }
    }
  }

  Err(
    "No Python framework found on the system. Searched in:\n".to_string()
      + &lib_paths[..10].join("\n  ")
      + "\n  ... and more",
  )
}

fn find_python_library() -> Result<String, String> {
  // Generate Python versions from 3.20 down to 3.8
  let mut python_versions = Vec::new();
  for major in (8..=20).rev() {
    // Standard versioned libraries
    python_versions.push(format!("libpython3.{major}.so.1.0"));
    python_versions.push(format!("libpython3.{major}.so.1"));
    python_versions.push(format!("libpython3.{major}.so"));
    // Some distributions include 'm' suffix
    python_versions.push(format!("libpython3.{major}m.so.1.0"));
    python_versions.push(format!("libpython3.{major}m.so.1"));
    python_versions.push(format!("libpython3.{major}m.so"));
  }

  eprintln!(
    "fix-python-soname: Looking for versions: {:?}",
    &python_versions[0..6]
  );

  // Get system architecture
  let arch = std::env::consts::ARCH;
  let arch_triplet = match arch {
    "x86_64" => "x86_64-linux-gnu",
    "x86" => "i386-linux-gnu",
    "aarch64" => "aarch64-linux-gnu",
    "arm" => "arm-linux-gnueabihf",
    "powerpc64" => "powerpc64le-linux-gnu",
    "s390x" => "s390x-linux-gnu",
    _ => "",
  };

  // Comprehensive list of library paths
  let mut lib_paths = vec![
    // Standard system paths (most common first)
    "/usr/lib",
    "/usr/lib64",
    "/usr/local/lib",
    "/usr/local/lib64",
    "/lib",
    "/lib64",
    "/lib32",
    // Debian/Ubuntu multiarch paths
    "/usr/lib/x86_64-linux-gnu",
    "/usr/lib/i386-linux-gnu",
    "/usr/lib/aarch64-linux-gnu",
    "/usr/lib/arm-linux-gnueabihf",
    // RedHat/CentOS/Fedora Software Collections
    "/opt/rh/rh-python38/root/usr/lib64",
    "/opt/rh/rh-python39/root/usr/lib64",
    "/opt/rh/rh-python310/root/usr/lib64",
    "/opt/rh/rh-python311/root/usr/lib64",
    // Python built from source
    "/usr/local/python/lib",
    "/usr/local/python3/lib",
    "/opt/python/lib",
    "/opt/python3/lib",
    "/opt/python-3.11/lib",
    "/opt/python-3.12/lib",
    // Container/Docker common paths
    "/usr/lib/python3",
    "/usr/local/lib/python3",
    "/opt/lib",
    // Snap packages
    "/snap/core18/current/usr/lib",
    "/snap/core20/current/usr/lib",
    "/snap/core22/current/usr/lib",
    "/snap/python38/current/usr/lib",
    "/snap/python39/current/usr/lib",
    "/snap/python310/current/usr/lib",
    // Flatpak runtime paths
    "/var/lib/flatpak/runtime/org.freedesktop.Platform/x86_64/21.08/active/files/lib",
    "/var/lib/flatpak/runtime/org.freedesktop.Platform/x86_64/22.08/active/files/lib",
    "/var/lib/flatpak/runtime/org.freedesktop.Platform/x86_64/23.08/active/files/lib",
    // Alpine Linux (musl)
    "/usr/lib/apk/db",
    // Homebrew on Linux
    "/home/linuxbrew/.linuxbrew/lib",
    "/opt/homebrew/lib",
    // macOS paths (for cross-platform support)
    "/System/Library/Frameworks/Python.framework/Versions/3.11/lib",
    "/System/Library/Frameworks/Python.framework/Versions/3.12/lib",
    "/Library/Frameworks/Python.framework/Versions/3.11/lib",
    "/Library/Frameworks/Python.framework/Versions/3.12/lib",
    // Nix/NixOS paths
    "/nix/var/nix/profiles/default/lib",
    "/run/current-system/sw/lib",
    // Gentoo
    "/usr/lib/python-exec/python3.11",
    "/usr/lib/python-exec/python3.12",
    // System Python config directories
    "/usr/lib/python3.8/config-3.8-x86_64-linux-gnu",
    "/usr/lib/python3.9/config-3.9-x86_64-linux-gnu",
    "/usr/lib/python3.10/config-3.10-x86_64-linux-gnu",
    "/usr/lib/python3.11/config-3.11-x86_64-linux-gnu",
    "/usr/lib/python3.12/config-3.12-x86_64-linux-gnu",
  ];

  // Add architecture-specific paths if we detected the architecture
  if !arch_triplet.is_empty() {
    lib_paths.insert(
      0,
      Box::leak(format!("/usr/lib/{}", arch_triplet).into_boxed_str()),
    );
    lib_paths.insert(
      1,
      Box::leak(format!("/usr/local/lib/{}", arch_triplet).into_boxed_str()),
    );
    lib_paths.insert(
      2,
      Box::leak(format!("/lib/{}", arch_triplet).into_boxed_str()),
    );
  }

  // Add conda/anaconda paths from common locations
  let conda_paths = vec![
    "/opt/conda/lib",
    "/opt/anaconda/lib",
    "/opt/anaconda3/lib",
    "/opt/miniconda/lib",
    "/opt/miniconda3/lib",
  ];
  lib_paths.extend(conda_paths);

  // Check for active virtual environments first
  if let Ok(venv) = env::var("VIRTUAL_ENV") {
    lib_paths.insert(0, Box::leak(format!("{}/lib", venv).into_boxed_str()));
    lib_paths.insert(0, Box::leak(format!("{}/lib64", venv).into_boxed_str()));
  }

  // Check for active conda environment
  if let Ok(conda_prefix) = env::var("CONDA_PREFIX") {
    lib_paths.insert(
      0,
      Box::leak(format!("{}/lib", conda_prefix).into_boxed_str()),
    );
  }

  // Add user-specific paths
  if let Ok(home) = env::var("HOME") {
    // Conda/Mamba installations
    lib_paths.push(Box::leak(
      format!("{}/anaconda3/lib", home).into_boxed_str(),
    ));
    lib_paths.push(Box::leak(
      format!("{}/miniconda3/lib", home).into_boxed_str(),
    ));
    lib_paths.push(Box::leak(
      format!("{}/miniforge3/lib", home).into_boxed_str(),
    ));
    lib_paths.push(Box::leak(
      format!("{}/mambaforge/lib", home).into_boxed_str(),
    ));
    lib_paths.push(Box::leak(
      format!("{}/.conda/envs/base/lib", home).into_boxed_str(),
    ));

    // Pyenv - search all installed versions
    let pyenv_root = format!("{}/.pyenv/versions", home);
    if let Ok(entries) = fs::read_dir(&pyenv_root) {
      for entry in entries.flatten() {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
          let version_lib = format!("{}/lib", entry.path().display());
          lib_paths.push(Box::leak(version_lib.into_boxed_str()));
        }
      }
    }

    // Local installations
    lib_paths.push(Box::leak(format!("{}/.local/lib", home).into_boxed_str()));
    lib_paths.push(Box::leak(format!("{}/.local/lib64", home).into_boxed_str()));

    // asdf version manager
    let asdf_python = format!("{}/.asdf/installs/python", home);
    if let Ok(entries) = fs::read_dir(&asdf_python) {
      for entry in entries.flatten() {
        if entry.file_type().map(|t| t.is_dir()).unwrap_or(false) {
          let version_lib = format!("{}/lib", entry.path().display());
          lib_paths.push(Box::leak(version_lib.into_boxed_str()));
        }
      }
    }
  }

  // Add paths from environment variables
  if let Ok(ld_library_path) = env::var("LD_LIBRARY_PATH") {
    for path in ld_library_path.split(':') {
      if !path.is_empty() {
        lib_paths.push(Box::leak(path.to_string().into_boxed_str()));
      }
    }
  }

  eprintln!(
    "fix-python-soname: Searching in {} directories...",
    lib_paths.len()
  );
  eprintln!(
    "fix-python-soname: First 5 paths: {:?}",
    &lib_paths[0..5.min(lib_paths.len())]
  );

  // First try exact version matches
  for lib_name in &python_versions {
    for lib_path in &lib_paths {
      let full_path = format!("{}/{}", lib_path, lib_name);
      if Path::new(&full_path).exists() {
        eprintln!(
          "fix-python-soname: Found Python library: {} at {}",
          lib_name, full_path
        );
        return Ok(lib_name.to_string());
      }
    }
  }

  eprintln!("fix-python-soname: No exact match found, searching for any libpython*.so files...");

  // If no exact match found, search directories for any libpython*.so
  for lib_path in &lib_paths {
    if let Ok(entries) = fs::read_dir(lib_path) {
      let mut found_libs: Vec<(String, u32, u32)> = Vec::new();

      for entry in entries.flatten() {
        if let Some(name) = entry.file_name().to_str() {
          if name.starts_with("libpython") && name.contains(".so") {
            // Try to extract version numbers
            if let Some(version_start) = name.find("3.") {
              let version_part = &name[version_start + 2..];
              if let Some(major_end) = version_part.find(|c: char| !c.is_numeric()) {
                if let Ok(minor) = version_part[..major_end].parse::<u32>() {
                  found_libs.push((name.to_string(), 3, minor));
                }
              }
            }
          }
        }
      }

      // Sort by version (newest first)
      found_libs.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.cmp(&a.1)));

      if let Some((lib_name, _, _)) = found_libs.first() {
        eprintln!(
          "fix-python-soname: Found Python library: {} in {}",
          lib_name, lib_path
        );
        return Ok(lib_name.clone());
      }
    }
  }

  Err(
    "No Python library found on the system. Searched in:\n".to_string()
      + &lib_paths[..10].join("\n  ")
      + "\n  ... and more",
  )
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
  eprintln!("fix-python-soname: Starting binary patcher...");

  let args: Vec<String> = env::args().collect();
  eprintln!("fix-python-soname: Arguments: {:?}", args);

  if args.len() != 2 {
    return Err(format!("Usage: {} <path-to-node-file>", args[0]).into());
  }

  let node_file_path = &args[1];
  eprintln!("fix-python-soname: Processing file: {}", node_file_path);

  // Read the file first to detect format
  eprintln!("fix-python-soname: Reading binary file...");
  let file_contents =
    fs::read(node_file_path).map_err(|error| format!("Failed to read file: {error}"))?;
  eprintln!(
    "fix-python-soname: Binary file size: {} bytes",
    file_contents.len()
  );

  // Detect binary format and process accordingly
  if is_elf_binary(&file_contents) {
    eprintln!("fix-python-soname: Detected ELF binary (Linux)");
    process_elf_binary(&file_contents, node_file_path)
  } else if is_macho_binary(&file_contents) {
    eprintln!("fix-python-soname: Detected Mach-O binary (macOS)");
    process_macho_binary(&file_contents, node_file_path)
  } else {
    Err("Unsupported binary format. Only ELF (Linux) and Mach-O (macOS) are supported.".into())
  }
}

fn process_elf_binary(
  file_contents: &[u8],
  node_file_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
  // Find the local Python library (Linux)
  let new_python_lib = find_python_library()?;

  // Parse the ELF file
  eprintln!("fix-python-soname: Parsing ELF file...");
  let mut elf =
    ElfContainer::parse(file_contents).map_err(|error| format!("Failed to parse ELF: {error}"))?;

  // Get the list of needed libraries
  eprintln!("fix-python-soname: Getting needed libraries...");
  let needed_libs: Vec<String> = elf
    .inner
    .elf_needed()
    .map(|lib| String::from_utf8_lossy(lib).to_string())
    .collect();

  eprintln!("fix-python-soname: Needed libraries: {:?}", needed_libs);

  // Find the existing Python dependency
  let python_lib = needed_libs
    .iter()
    .find(|lib| lib.starts_with("libpython") && lib.contains(".so"))
    .ok_or("No Python library dependency found in the binary")?;

  eprintln!(
    "fix-python-soname: Current Python dependency: {}",
    python_lib
  );

  // Check if already pointing to the correct library
  if python_lib == &new_python_lib {
    eprintln!("fix-python-soname: Already using the correct Python library");
    return Ok(());
  }

  eprintln!("fix-python-soname: Replacing with: {}", new_python_lib);

  // Create a map for replacement
  let mut replacements = HashMap::new();
  replacements.insert(python_lib.clone(), new_python_lib);

  // Replace the needed dependency
  eprintln!("fix-python-soname: Replacing dependency...");
  elf
    .replace_needed(&replacements)
    .map_err(|error| format!("Failed to replace needed dependency: {error}"))?;

  // Create backup
  let file_path = Path::new(node_file_path);
  let backup_path = file_path.with_extension("node.bak");
  eprintln!(
    "fix-python-soname: Creating backup at: {}",
    backup_path.display()
  );
  fs::copy(file_path, &backup_path).map_err(|error| format!("Failed to create backup: {error}"))?;
  eprintln!("fix-python-soname: Backup created successfully");

  // Write the modified file
  eprintln!("fix-python-soname: Writing modified ELF file...");
  let output_file = File::create(node_file_path)
    .map_err(|error| format!("Failed to create output file: {error}"))?;

  elf
    .write(&output_file)
    .map_err(|error| format!("Failed to write ELF: {error}"))?;

  eprintln!(
    "fix-python-soname: Successfully updated: {}",
    node_file_path
  );

  Ok(())
}

fn process_macho_binary(
  file_contents: &[u8],
  node_file_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
  // Find the local Python framework (macOS)
  let new_python_framework = find_python_library_macos()?;

  // Parse the Mach-O file
  eprintln!("fix-python-soname: Parsing Mach-O file...");
  let mut macho = MachoContainer::parse(file_contents)
    .map_err(|error| format!("Failed to parse Mach-O: {error}"))?;

  // Get the list of linked libraries (equivalent to needed libs on ELF)
  eprintln!("fix-python-soname: Getting linked libraries...");

  // Access the libs field based on the macho type
  let libs = match &macho.inner {
    arwen::macho::MachoType::SingleArch(single) => &single.inner.libs,
    arwen::macho::MachoType::Fat(fat) => {
      if fat.archs.is_empty() {
        return Err("No architectures found in fat binary".into());
      }
      &fat.archs[0].inner.inner.libs // Use first architecture
    }
  };

  eprintln!("fix-python-soname: Linked libraries: {:?}", libs);

  // Find the existing Python framework dependency
  let python_framework = libs
    .iter()
    .find(|lib| lib.contains("Python.framework") || lib.contains("Python"))
    .ok_or("No Python framework dependency found in the binary")?;

  eprintln!(
    "fix-python-soname: Current Python framework: {}",
    python_framework
  );

  // Check if already pointing to the correct framework
  if python_framework == &new_python_framework {
    eprintln!("fix-python-soname: Already using the correct Python framework");
    return Ok(());
  }

  eprintln!(
    "fix-python-soname: Replacing with: {}",
    new_python_framework
  );

  // Use change_install_name to replace the Python framework path
  eprintln!("fix-python-soname: Changing install name...");
  macho
    .change_install_name(python_framework, &new_python_framework)
    .map_err(|error| format!("Failed to change install name: {error}"))?;

  // Create backup
  let file_path = Path::new(node_file_path);
  let backup_path = file_path.with_extension("node.bak");
  eprintln!(
    "fix-python-soname: Creating backup at: {}",
    backup_path.display()
  );
  fs::copy(file_path, &backup_path).map_err(|error| format!("Failed to create backup: {error}"))?;
  eprintln!("fix-python-soname: Backup created successfully");

  // Write the modified file
  eprintln!("fix-python-soname: Writing modified Mach-O file...");
  fs::write(node_file_path, &macho.data)
    .map_err(|error| format!("Failed to write Mach-O: {error}"))?;

  eprintln!(
    "fix-python-soname: Successfully updated: {}",
    node_file_path
  );

  Ok(())
}
