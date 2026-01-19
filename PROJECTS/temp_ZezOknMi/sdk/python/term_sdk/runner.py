"""
Agent runner for Term Challenge SDK 2.0.

HTTP Server that orchestrates agent execution:
- POST /start - Start task execution with instruction
- GET /status - Get current execution status
- GET /health - Health check

The agent runs in a background thread, executing commands via subprocess.
The harness polls /status until completion or timeout.

Example:
    ```python
    from term_sdk import Agent, AgentContext, run
    
    class MyAgent(Agent):
        def run(self, ctx: AgentContext):
            ctx.shell("ls -la")
            ctx.done()
    
    if __name__ == "__main__":
        run(MyAgent())
    ```
"""

import sys
import json
import traceback
import time
import os
import signal
import threading
from http.server import HTTPServer, BaseHTTPRequestHandler
from socketserver import ThreadingMixIn
from typing import Optional, Dict, Any, List

from .agent import Agent, AgentContext

# Import CostLimitExceeded if available
try:
    from .llm import CostLimitExceeded as _CostLimitExceeded
    CostLimitExceeded = _CostLimitExceeded
except ImportError:
    class _FallbackCostLimitExceeded(Exception):
        """Placeholder if LLM module not available."""
        pass
    CostLimitExceeded = _FallbackCostLimitExceeded


# ============================================================================
# Configuration
# ============================================================================

DEFAULT_PORT = 8765

# ============================================================================
# Logging
# ============================================================================

_log_enabled = True


def set_logging(enabled: bool) -> None:
    """Enable or disable runner logging."""
    global _log_enabled
    _log_enabled = enabled


def log(msg: str) -> None:
    """Log a message to stderr with timestamp."""
    if _log_enabled:
        timestamp = time.strftime("%H:%M:%S")
        print(f"[{timestamp}] [runner] {msg}", file=sys.stderr, flush=True)


def log_error(msg: str) -> None:
    """Log an error message to stderr with timestamp."""
    timestamp = time.strftime("%H:%M:%S")
    print(f"[{timestamp}] [runner] ERROR: {msg}", file=sys.stderr, flush=True)


def log_step(step: int, msg: str) -> None:
    """Log a step-related message."""
    if _log_enabled:
        timestamp = time.strftime("%H:%M:%S")
        print(f"[{timestamp}] [step {step}] {msg}", file=sys.stderr, flush=True)


# ============================================================================
# Agent Runner
# ============================================================================

class AgentRunner:
    """
    Manages agent execution state and lifecycle.
    
    States:
        - idle: Waiting for /start
        - running: Agent is executing
        - completed: Agent finished successfully
        - failed: Agent failed with error
    """
    
    def __init__(self, agent: Agent):
        self.agent = agent
        self.status = "idle"
        self.ctx: Optional[AgentContext] = None
        self.error: Optional[str] = None
        self.thread: Optional[threading.Thread] = None
        self.start_time: Optional[float] = None
        self._lock = threading.Lock()
    
    def start(self, instruction: str) -> bool:
        """
        Start agent execution in background thread.
        
        Args:
            instruction: Task instruction
        
        Returns:
            True if started, False if already running
        """
        with self._lock:
            if self.status == "running":
                return False
            
            self.status = "running"
            self.error = None
            self.start_time = time.time()
            self.ctx = AgentContext(instruction=instruction)
            
            self.thread = threading.Thread(target=self._run_agent, daemon=True)
            self.thread.start()
            return True
    
    def _run_agent(self) -> None:
        """Execute agent in thread."""
        ctx = self.ctx
        if ctx is None:
            self.status = "failed"
            self.error = "Context not initialized"
            return
        
        try:
            log("Starting agent.run()...")
            self.agent.run(ctx)
            
            # Auto-complete if agent forgot to call done()
            if not ctx.is_done:
                log("Agent did not call ctx.done(), auto-completing")
                ctx.done()
            
            self.status = "completed"
            log(f"Agent completed in {ctx.step} steps, {ctx.elapsed_secs:.1f}s")
            
        except CostLimitExceeded as e:
            self.status = "failed"
            self.error = f"Cost limit exceeded: {e}"
            log_error(self.error)
            
        except RuntimeError as e:
            # Max steps or timeout exceeded
            self.status = "failed" 
            self.error = str(e)
            log_error(f"Agent limit exceeded: {e}")
            
        except Exception as e:
            self.status = "failed"
            self.error = str(e)
            log_error(f"Agent failed: {e}")
            traceback.print_exc(file=sys.stderr)
    
    def get_status(self) -> Dict[str, Any]:
        """Get current status as dict for JSON response."""
        elapsed = 0
        if self.start_time:
            elapsed = int(time.time() - self.start_time)
        
        # Build history (last 30 entries, truncated output)
        history: List[Dict[str, Any]] = []
        if self.ctx:
            for h in self.ctx.history[-30:]:
                # Combine stdout + stderr for output
                output = h.stdout or ""
                if h.stderr:
                    output = output + "\n" + h.stderr if output else h.stderr
                history.append({
                    "step": h.step,
                    "command": h.command[:200] if h.command else None,
                    "output": output[:500] if output else None,
                    "exit_code": h.exit_code,
                })
        
        return {
            "status": self.status,
            "steps": self.ctx.step if self.ctx else 0,
            "elapsed_secs": elapsed,
            "error": self.error,
            "done": self.ctx.is_done if self.ctx else False,
            "history": history,
        }


# ============================================================================
# HTTP Handler
# ============================================================================

# Global runner instance
_runner: Optional[AgentRunner] = None


class ThreadingHTTPServer(ThreadingMixIn, HTTPServer):
    """HTTP server that handles requests in separate threads."""
    daemon_threads = True


class AgentHandler(BaseHTTPRequestHandler):
    """HTTP request handler for agent communication."""
    
    protocol_version = "HTTP/1.1"
    
    def log_message(self, format: str, *args) -> None:
        """Override to use our logging format. Skip /health and /status to reduce noise."""
        msg = format % args if args else format
        if '/health' not in msg and '/status' not in msg:
            log(f"HTTP: {msg}")
    
    def do_POST(self) -> None:
        """Handle POST requests."""
        if self.path == '/start':
            self._handle_start()
        elif self.path == '/health':
            self._send_json(200, {"status": "ok"})
        else:
            self._send_json(404, {"error": "not found"})
    
    def do_GET(self) -> None:
        """Handle GET requests."""
        if self.path == '/health':
            self._send_json(200, {"status": "ok"})
        elif self.path == '/status':
            self._handle_status()
        else:
            self._send_json(404, {"error": "not found"})
    
    def _handle_start(self) -> None:
        """Handle POST /start request."""
        global _runner
        
        try:
            content_length = int(self.headers.get('Content-Length', 0))
            body = self.rfile.read(content_length).decode('utf-8')
            data = json.loads(body) if body else {}
            
            instruction = data.get("instruction", "")
            
            if not instruction:
                self._send_json(400, {"error": "instruction required"})
                return
            
            log(f"Received /start: {len(instruction)} chars")
            log(f"Instruction preview: {instruction[:200]}...")
            
            if _runner is None:
                self._send_json(500, {"error": "runner not initialized"})
                return
            
            if _runner.start(instruction):
                self._send_json(200, {"status": "started"})
            else:
                self._send_json(409, {"error": "already running"})
                
        except json.JSONDecodeError as e:
            log_error(f"Invalid JSON in /start: {e}")
            self._send_json(400, {"error": f"invalid JSON: {e}"})
            
        except Exception as e:
            log_error(f"Error in /start: {e}")
            traceback.print_exc(file=sys.stderr)
            self._send_json(500, {"error": str(e)})
    
    def _handle_status(self) -> None:
        """Handle GET /status request."""
        global _runner
        
        if _runner is None:
            self._send_json(500, {"error": "runner not initialized"})
            return
        
        status = _runner.get_status()
        self._send_json(200, status)
    
    def _send_json(self, code: int, data: Dict[str, Any]) -> None:
        """Send JSON response."""
        body = json.dumps(data).encode('utf-8')
        self.send_response(code)
        self.send_header('Content-Type', 'application/json')
        self.send_header('Content-Length', str(len(body)))
        self.end_headers()
        self.wfile.write(body)


# ============================================================================
# Main Entry Point
# ============================================================================

def run(agent: Agent, port: Optional[int] = None) -> None:
    """
    Run an agent as HTTP server (SDK 2.0).
    
    The agent will:
    1. Call setup() once at startup
    2. Listen for POST /start with instruction
    3. Execute run(ctx) in background thread
    4. Respond to GET /status with progress
    5. Call cleanup() on shutdown
    
    Args:
        agent: Your Agent instance with run() implemented
        port: HTTP port (default: 8765, or AGENT_PORT env var)
    
    Example:
        ```python
        from term_sdk import Agent, AgentContext, run
        
        class MyAgent(Agent):
            def run(self, ctx: AgentContext):
                result = ctx.shell("ls -la")
                ctx.log(f"Output: {result.output[:100]}")
                ctx.done()
        
        if __name__ == "__main__":
            run(MyAgent())
        ```
    """
    global _runner
    
    if port is None:
        port = int(os.environ.get('AGENT_PORT', DEFAULT_PORT))
    
    # Print startup banner
    log("=" * 60)
    log("TERM SDK 2.0 - Agent Starting")
    log("=" * 60)
    log(f"Python version: {sys.version.split()[0]}")
    log(f"Platform: {sys.platform}")
    log(f"Working directory: {os.getcwd()}")
    log(f"HTTP port: {port}")
    log("-" * 60)
    
    # Log relevant environment variables
    env_vars = ['LLM_PROXY_URL', 'TERM_AGENT_HASH', 'TERM_TASK_ID', 'EVALUATION_MODE']
    for var in env_vars:
        value = os.environ.get(var)
        if value:
            display_value = value[:50] + "..." if len(value) > 50 else value
            log(f"  {var}={display_value}")
    
    log("-" * 60)
    
    try:
        # Initialize agent
        log("Calling agent.setup()...")
        start_setup = time.time()
        agent.setup()
        setup_time = int((time.time() - start_setup) * 1000)
        log(f"Setup complete ({setup_time}ms)")
        
        # Create runner
        _runner = AgentRunner(agent)
        
        # Create and start HTTP server (threaded for concurrent requests)
        server = ThreadingHTTPServer(('0.0.0.0', port), AgentHandler)
        log(f"HTTP server listening on 0.0.0.0:{port}")
        log("Waiting for /start request...")
        log("=" * 60)
        
        # Setup signal handlers for graceful shutdown
        def shutdown_handler(signum: int, frame) -> None:
            sig_name = signal.Signals(signum).name if hasattr(signal, 'Signals') else str(signum)
            log(f"Received {sig_name}, initiating shutdown...")
            server.shutdown()
        
        signal.signal(signal.SIGTERM, shutdown_handler)
        signal.signal(signal.SIGINT, shutdown_handler)
        
        # Start serving requests
        server.serve_forever()
        
    except KeyboardInterrupt:
        log("Interrupted by signal")
        
    except Exception as e:
        log_error(f"Fatal error: {e}")
        traceback.print_exc(file=sys.stderr)
        sys.exit(1)
        
    finally:
        log("-" * 60)
        log("Shutting down...")
        try:
            agent.cleanup()
            log("Cleanup complete")
        except Exception as e:
            log_error(f"Error during cleanup: {e}")
        log("Agent finished")
        log("=" * 60)
