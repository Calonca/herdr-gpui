# Mods that could not run or be listed. $name is a mod's name, as its
# mod.toml gives it; `[mods] disabled` is a config table and key.

error-mods-busy = Mod { $name } has too many messages waiting; try again
error-mods-disabled-name = `[mods] disabled` has an invalid name `{ $name }`
error-mods-not-running = No mod named { $name } is running
error-mods-spawn = Could not start a thread for mod { $name }
# $limit is the most mods the list may name.
error-mods-too-many-disabled = `[mods] disabled` lists more than { $limit } mods
# A WASM mod that could not be shown. wasm-mods-host is the program that
# runs WASM mods (a name, not translated); $version is its version, $port a
# TCP port number, and $protocol the version of its protocol.
error-mods-wasm-closed = wasm-mods-host closed the connection before it was ready
error-mods-wasm-connect = Could not reach wasm-mods-host on port { $port }
error-mods-wasm-exited = wasm-mods-host { $version } stopped before it was ready
# $message is the host's own explanation, in English.
error-mods-wasm-host = wasm-mods-host: { $message }
error-mods-wasm-io = Could not read or write { $path }
error-mods-wasm-no-home = No home folder to find wasm-mods-host in
error-mods-wasm-no-host = No installed wasm-mods-host runs protocol { $protocol }. Install WASM Mods 0.2.0 or later.
error-mods-wasm-random = Could not make a token for wasm-mods-host
error-mods-wasm-start = Could not start wasm-mods-host { $version }
error-mods-wasm-starting = wasm-mods-host is still starting in another window
error-mods-wasm-thread = Could not start a thread for a WASM mod
# $limit is a size in bytes.
error-mods-wasm-too-large = The mod's module is larger than { $limit } bytes
error-mods-wasm-slow = wasm-mods-host { $version } did not start in time
