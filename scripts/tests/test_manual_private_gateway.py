"""Local-only mode uses artificial text in tests; never loads private fixtures."""
import sys
import tempfile
import unittest
from pathlib import Path
sys.path.insert(0,str(Path(__file__).resolve().parents[1]))
from manual_model_gateway import ManualGateway, read, write, ENDPOINT, SYNTHETIC_KEY
class PrivateQueueTests(unittest.TestCase):
 def test_explicit_mode_and_reopen(self):
  with tempfile.TemporaryDirectory() as d:
   q=ManualGateway(d,private_local_only=True)
   self.assertTrue(q.control()['private_local_only']);self.assertFalse(q.control()['synthetic_only'])
   self.assertTrue(ManualGateway(d,create=False).control()['private_local_only'])
   with self.assertRaises(PermissionError):ManualGateway(d)
 def test_modes_are_exclusive(self):
  with tempfile.TemporaryDirectory() as d:
   q=ManualGateway(d,private_local_only=True); p=Path(d)/'control.json';c=read(p);c['synthetic_only']=True;p.write_text(__import__('json').dumps(c))
   with self.assertRaises(PermissionError):q.control()
 def test_live_mode_change_revokes_existing_reader(self):
  with tempfile.TemporaryDirectory() as d:
   q=ManualGateway(d,private_local_only=True);p=Path(d)/'control.json';c=read(p)
   c['synthetic_only']=True;c['private_local_only']=False;p.write_text(__import__('json').dumps(c))
   with self.assertRaises(PermissionError):q.control()
 def test_private_queue_rejects_external_destination(self):
  with tempfile.TemporaryDirectory() as d:
   q=ManualGateway(d,private_local_only=True)
   with self.assertRaises(PermissionError):q('https://example.com',b'{}',SYNTHETIC_KEY,1,100)
   self.assertEqual(list((Path(d)/'requests').iterdir()),[])
if __name__=='__main__':unittest.main()
